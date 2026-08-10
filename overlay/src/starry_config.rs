use once_cell::sync::Lazy;
use serde_derive::Deserialize;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

pub const DEFAULT_CONFIG_PATH: &str = "starry/config.yaml";
const MIN_CONFIG_VERSION: u8 = 1;
const CONFIG_VERSION: u8 = 2;
const EXAMPLE_CONFIG: &str = include_str!("starry_config.example.yaml");

static STATE: Lazy<RwLock<ConfigState>> = Lazy::new(|| {
    RwLock::new(ConfigState {
        path: PathBuf::from(DEFAULT_CONFIG_PATH),
        config: None,
    })
});

struct ConfigState {
    path: PathBuf,
    config: Option<Arc<StarryConfig>>,
}

#[derive(Clone, Debug)]
pub struct StarryConfig {
    pub version: u8,
    pub relay_servers: Vec<String>,
    pub secure_tcp: SecureTcpConfig,
    pub mmdb: MmdbConfig,
    pub geo: GeoConfig,
    pub websocket_signal: WebSocketSignalConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StarryConfigWire {
    version: u8,
    #[serde(default)]
    relay_servers: Vec<String>,
    #[serde(default)]
    secure_tcp: SecureTcpConfig,
    #[serde(default)]
    mmdb: MmdbConfig,
    #[serde(default)]
    geo: GeoConfig,
    #[serde(default)]
    websocket_signal: Option<WebSocketSignalConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SecureTcpMode {
    Off,
    Auto,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecureTcpConfig {
    pub mode: SecureTcpMode,
    pub handshake_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub max_frame_bytes: usize,
}

impl Default for SecureTcpConfig {
    fn default() -> Self {
        Self {
            mode: SecureTcpMode::Off,
            handshake_timeout_ms: 18_000,
            idle_timeout_ms: 30_000,
            max_frame_bytes: 65_536,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WebSocketSignalConfig {
    pub enabled: bool,
    pub registration_timeout_ms: u64,
    pub keepalive_interval_ms: u64,
    pub idle_timeout_ms: u64,
    pub max_frame_bytes: usize,
    pub outbound_queue_capacity: usize,
    pub max_sessions: usize,
    pub max_sessions_per_effective_ip: usize,
    pub registration_rate_per_minute: usize,
    pub trusted_proxies: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub relay_health: RelayHealthConfig,
}

impl Default for WebSocketSignalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            registration_timeout_ms: 10_000,
            keepalive_interval_ms: 12_000,
            idle_timeout_ms: 45_000,
            max_frame_bytes: 65_536,
            outbound_queue_capacity: 64,
            max_sessions: 10_000,
            max_sessions_per_effective_ip: 512,
            registration_rate_per_minute: 300,
            trusted_proxies: vec!["127.0.0.1/32".to_owned(), "::1/128".to_owned()],
            allowed_origins: Vec::new(),
            relay_health: RelayHealthConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RelayHealthConfig {
    pub interval_seconds: u64,
    pub timeout_ms: u64,
    pub success_threshold: u32,
    pub failure_threshold: u32,
    pub endpoints: Vec<RelayEndpointConfig>,
}

impl Default for RelayHealthConfig {
    fn default() -> Self {
        Self {
            interval_seconds: 60,
            timeout_ms: 5_000,
            success_threshold: 1,
            failure_threshold: 2,
            endpoints: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RelayEndpointConfig {
    pub relay: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MmdbConfig {
    pub update_interval_hours: u64,
    pub update_on_start: bool,
    pub force_update: bool,
    pub download_timeout_seconds: u64,
    pub minimum_bytes: u64,
    pub country: DatabaseConfig,
    pub city: DatabaseConfig,
    pub asn: DatabaseConfig,
}

impl Default for MmdbConfig {
    fn default() -> Self {
        Self {
            update_interval_hours: 168,
            update_on_start: true,
            force_update: false,
            download_timeout_seconds: 600,
            minimum_bytes: 65_536,
            country: DatabaseConfig::with_path("mmdb/GeoLite2-Country.mmdb"),
            city: DatabaseConfig::with_path("mmdb/GeoLite2-City.mmdb"),
            asn: DatabaseConfig::with_path("mmdb/GeoLite2-ASN.mmdb"),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: String,
    pub url: String,
}

impl DatabaseConfig {
    fn with_path(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            url: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeoConfig {
    pub enabled: bool,
    pub rules: Vec<GeoRuleConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoRuleConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub symmetric: bool,
    #[serde(rename = "match")]
    pub matches: EndpointExpressions,
    pub relays: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EndpointExpressions {
    pub client_a: String,
    pub client_b: String,
}

impl Default for EndpointExpressions {
    fn default() -> Self {
        Self {
            client_a: "*".to_owned(),
            client_b: "*".to_owned(),
        }
    }
}

pub struct ReloadOutcome {
    pub message: String,
    pub relay_servers: Option<String>,
}

pub fn initialize(path: &str) -> ReloadOutcome {
    let path = normalized_path(path);
    let artifact_message = ensure_artifacts(&path);
    apply_loaded(path, artifact_message)
}

pub fn reload() -> ReloadOutcome {
    let path = match STATE.read() {
        Ok(state) => state.path.clone(),
        Err(err) => {
            return ReloadOutcome {
                message: format!("Starry config lock failed: {err}"),
                relay_servers: None,
            }
        }
    };
    apply_loaded(path, Vec::new())
}

pub fn snapshot() -> Option<Arc<StarryConfig>> {
    STATE.read().ok()?.config.clone()
}

fn apply_loaded(path: PathBuf, mut artifact_messages: Vec<String>) -> ReloadOutcome {
    let loaded = load_config(&path);
    let (config, load_message) = match loaded {
        Ok(Some(config)) => {
            let relay_count = config.relay_servers.len();
            let rule_count = config.geo.rules.len();
            (
                Some(Arc::new(config)),
                format!(
                    "Starry config loaded from {}: {relay_count} relays, {rule_count} Geo rules",
                    path.display()
                ),
            )
        }
        Ok(None) => (
            None,
            format!(
                "Starry config {} is empty; using upstream behavior",
                path.display()
            ),
        ),
        Err(err) => (
            None,
            format!(
                "Starry config {} is invalid; using upstream behavior: {err}",
                path.display()
            ),
        ),
    };

    let relay_servers = config.as_ref().and_then(|config| {
        if config.relay_servers.is_empty() {
            None
        } else {
            Some(config.relay_servers.join(","))
        }
    });
    match STATE.write() {
        Ok(mut state) => {
            state.path = path;
            state.config = config;
        }
        Err(err) => {
            artifact_messages.push(format!("Starry config state lock failed: {err}"));
        }
    }
    artifact_messages.push(load_message);

    ReloadOutcome {
        message: artifact_messages.join("; "),
        relay_servers,
    }
}

fn normalized_path(path: &str) -> PathBuf {
    let path = path.trim();
    if path.is_empty() {
        PathBuf::from(DEFAULT_CONFIG_PATH)
    } else {
        PathBuf::from(path)
    }
}

fn ensure_artifacts(config_path: &Path) -> Vec<String> {
    let mut messages = Vec::new();
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = fs::create_dir_all(parent) {
        messages.push(format!(
            "cannot create Starry config directory {}: {err}",
            parent.display()
        ));
        return messages;
    }

    if !config_path.exists() {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(config_path)
        {
            Ok(_) => messages.push(format!(
                "created empty Starry config {}",
                config_path.display()
            )),
            Err(err) => messages.push(format!(
                "cannot create empty Starry config {}: {err}",
                config_path.display()
            )),
        }
    }

    let example_path = parent.join("config.example.yaml");
    if !example_path.exists() {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&example_path)
        {
            Ok(mut file) => match file.write_all(EXAMPLE_CONFIG.as_bytes()) {
                Ok(()) => messages.push(format!(
                    "created Starry example config {}",
                    example_path.display()
                )),
                Err(err) => messages.push(format!(
                    "cannot write Starry example config {}: {err}",
                    example_path.display()
                )),
            },
            Err(err) => messages.push(format!(
                "cannot create Starry example config {}: {err}",
                example_path.display()
            )),
        }
    }
    messages
}

fn load_config(path: &Path) -> Result<Option<StarryConfig>, String> {
    let raw =
        fs::read_to_string(path).map_err(|err| format!("cannot read configuration: {err}"))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_config(&raw).map(Some)
}

fn parse_config(raw: &str) -> Result<StarryConfig, String> {
    let wire: StarryConfigWire =
        serde_yml::from_str(raw).map_err(|err| format!("invalid YAML: {err}"))?;
    if !(MIN_CONFIG_VERSION..=CONFIG_VERSION).contains(&wire.version) {
        return Err(format!(
            "unsupported version {}; expected {MIN_CONFIG_VERSION} or {CONFIG_VERSION}",
            wire.version
        ));
    }
    if wire.version == 1 && wire.websocket_signal.is_some() {
        return Err(
            "version 1 does not allow websocket_signal; upgrade the document to version 2"
                .to_owned(),
        );
    }
    let config = StarryConfig {
        version: wire.version,
        relay_servers: wire.relay_servers,
        secure_tcp: wire.secure_tcp,
        mmdb: wire.mmdb,
        geo: wire.geo,
        websocket_signal: wire.websocket_signal.unwrap_or_default(),
    };
    validate(config)
}

fn validate(mut config: StarryConfig) -> Result<StarryConfig, String> {
    if !(MIN_CONFIG_VERSION..=CONFIG_VERSION).contains(&config.version) {
        return Err(format!(
            "unsupported version {}; expected {MIN_CONFIG_VERSION} or {CONFIG_VERSION}",
            config.version
        ));
    }

    normalize_unique(&mut config.relay_servers, "relay_servers")?;
    validate_secure_tcp(&config.secure_tcp)?;
    validate_mmdb(&mut config.mmdb)?;
    validate_geo(&mut config.geo, &config.relay_servers)?;
    validate_websocket_signal(&mut config.websocket_signal, &config.relay_servers)?;
    Ok(config)
}

fn validate_secure_tcp(config: &SecureTcpConfig) -> Result<(), String> {
    if !(1_000..=120_000).contains(&config.handshake_timeout_ms) {
        return Err("secure_tcp.handshake_timeout_ms must be between 1000 and 120000".to_owned());
    }
    if !(1_000..=600_000).contains(&config.idle_timeout_ms) {
        return Err("secure_tcp.idle_timeout_ms must be between 1000 and 600000".to_owned());
    }
    if !(4_096..=16 * 1024 * 1024).contains(&config.max_frame_bytes) {
        return Err("secure_tcp.max_frame_bytes must be between 4096 and 16777216".to_owned());
    }
    Ok(())
}

fn validate_websocket_signal(
    config: &mut WebSocketSignalConfig,
    relay_servers: &[String],
) -> Result<(), String> {
    if !(1_000..=120_000).contains(&config.registration_timeout_ms) {
        return Err(
            "websocket_signal.registration_timeout_ms must be between 1000 and 120000".to_owned(),
        );
    }
    if !(1_000..=300_000).contains(&config.keepalive_interval_ms) {
        return Err(
            "websocket_signal.keepalive_interval_ms must be between 1000 and 300000".to_owned(),
        );
    }
    if !(2_000..=600_000).contains(&config.idle_timeout_ms) {
        return Err("websocket_signal.idle_timeout_ms must be between 2000 and 600000".to_owned());
    }
    if config.keepalive_interval_ms >= config.idle_timeout_ms {
        return Err(
            "websocket_signal.keepalive_interval_ms must be smaller than idle_timeout_ms"
                .to_owned(),
        );
    }
    if !(4_096..=16 * 1024 * 1024).contains(&config.max_frame_bytes) {
        return Err(
            "websocket_signal.max_frame_bytes must be between 4096 and 16777216".to_owned(),
        );
    }
    if !(1..=4_096).contains(&config.outbound_queue_capacity) {
        return Err(
            "websocket_signal.outbound_queue_capacity must be between 1 and 4096".to_owned(),
        );
    }
    if !(1..=1_000_000).contains(&config.max_sessions) {
        return Err("websocket_signal.max_sessions must be between 1 and 1000000".to_owned());
    }
    if config.max_sessions_per_effective_ip == 0
        || config.max_sessions_per_effective_ip > config.max_sessions
    {
        return Err(
            "websocket_signal.max_sessions_per_effective_ip must be between 1 and max_sessions"
                .to_owned(),
        );
    }
    if !(1..=100_000).contains(&config.registration_rate_per_minute) {
        return Err(
            "websocket_signal.registration_rate_per_minute must be between 1 and 100000".to_owned(),
        );
    }

    normalize_unique(
        &mut config.trusted_proxies,
        "websocket_signal.trusted_proxies",
    )?;
    for cidr in &config.trusted_proxies {
        cidr.parse::<ipnetwork::IpNetwork>().map_err(|err| {
            format!("websocket_signal.trusted_proxies contains invalid CIDR '{cidr}': {err}")
        })?;
    }
    normalize_unique(
        &mut config.allowed_origins,
        "websocket_signal.allowed_origins",
    )?;
    for origin in &config.allowed_origins {
        let parsed = url::Url::parse(origin).map_err(|err| {
            format!("websocket_signal.allowed_origins contains invalid origin '{origin}': {err}")
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(format!(
                "websocket_signal.allowed_origins entry '{origin}' must be an exact http(s) origin"
            ));
        }
    }

    let health = &mut config.relay_health;
    if !(5..=3_600).contains(&health.interval_seconds) {
        return Err(
            "websocket_signal.relay_health.interval_seconds must be between 5 and 3600".to_owned(),
        );
    }
    if !(500..=120_000).contains(&health.timeout_ms) {
        return Err(
            "websocket_signal.relay_health.timeout_ms must be between 500 and 120000".to_owned(),
        );
    }
    if !(1..=100).contains(&health.success_threshold)
        || !(1..=100).contains(&health.failure_threshold)
    {
        return Err(
            "websocket_signal.relay_health thresholds must be between 1 and 100".to_owned(),
        );
    }

    let mut relays = HashSet::new();
    let mut urls = HashSet::new();
    for (index, endpoint) in health.endpoints.iter_mut().enumerate() {
        endpoint.relay = endpoint.relay.trim().to_owned();
        endpoint.url = endpoint.url.trim().to_owned();
        if endpoint.relay.is_empty() || endpoint.url.is_empty() {
            return Err(format!(
                "websocket_signal.relay_health.endpoints[{index}] has an empty relay or url"
            ));
        }
        if !relays.insert(endpoint.relay.to_ascii_lowercase()) {
            return Err(format!(
                "websocket_signal.relay_health contains duplicate relay '{}'",
                endpoint.relay
            ));
        }
        if !urls.insert(endpoint.url.to_ascii_lowercase()) {
            return Err(format!(
                "websocket_signal.relay_health contains duplicate URL '{}'",
                endpoint.url
            ));
        }
        let parsed = url::Url::parse(&endpoint.url).map_err(|err| {
            format!(
                "websocket_signal.relay_health endpoint '{}' has invalid URL: {err}",
                endpoint.relay
            )
        })?;
        if parsed.scheme() != "wss"
            || !matches!(parsed.host(), Some(url::Host::Domain(_)))
            || parsed.path() != "/ws/relay"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(format!(
                "websocket_signal.relay_health endpoint '{}' must use a hostname and exact wss://.../ws/relay URL without credentials, query, or fragment",
                endpoint.relay
            ));
        }
    }

    if config.enabled {
        if relay_servers.is_empty() {
            return Err("websocket_signal.enabled is true but relay_servers is empty".to_owned());
        }
        let configured: HashSet<String> = relay_servers
            .iter()
            .map(|relay| relay.to_ascii_lowercase())
            .collect();
        if relays != configured {
            let missing: Vec<&String> = relay_servers
                .iter()
                .filter(|relay| !relays.contains(&relay.to_ascii_lowercase()))
                .collect();
            let unknown: Vec<&String> = health
                .endpoints
                .iter()
                .map(|endpoint| &endpoint.relay)
                .filter(|relay| !configured.contains(&relay.to_ascii_lowercase()))
                .collect();
            return Err(format!(
                "websocket_signal.relay_health endpoints must cover relay_servers exactly; missing={missing:?}, unknown={unknown:?}"
            ));
        }
    }
    Ok(())
}

fn validate_mmdb(config: &mut MmdbConfig) -> Result<(), String> {
    if config.update_interval_hours > 8_760 {
        return Err("mmdb.update_interval_hours must not exceed 8760".to_owned());
    }
    if config.download_timeout_seconds == 0 || config.download_timeout_seconds > 3_600 {
        return Err("mmdb.download_timeout_seconds must be between 1 and 3600".to_owned());
    }
    if config.minimum_bytes < 1_024 || config.minimum_bytes > 1024 * 1024 * 1024 {
        return Err("mmdb.minimum_bytes must be between 1024 and 1073741824".to_owned());
    }
    for (label, database) in [
        ("country", &mut config.country),
        ("city", &mut config.city),
        ("asn", &mut config.asn),
    ] {
        database.path = database.path.trim().to_owned();
        database.url = database.url.trim().to_owned();
        if database.path.is_empty() {
            return Err(format!("mmdb.{label}.path must not be empty"));
        }
        if !database.url.is_empty()
            && !database.url.starts_with("https://")
            && !database.url.starts_with("http://")
        {
            return Err(format!("mmdb.{label}.url must use http:// or https://"));
        }
    }
    Ok(())
}

fn validate_geo(config: &mut GeoConfig, relay_servers: &[String]) -> Result<(), String> {
    if !config.enabled {
        return Ok(());
    }
    if config.rules.is_empty() {
        return Err("geo.enabled is true but geo.rules is empty".to_owned());
    }
    if relay_servers.is_empty() {
        return Err("geo.enabled is true but relay_servers is empty".to_owned());
    }

    let relay_set: HashSet<String> = relay_servers
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let mut names = HashSet::new();
    for (index, rule) in config.rules.iter_mut().enumerate() {
        rule.name = rule.name.trim().to_owned();
        if rule.name.is_empty() {
            return Err(format!("geo.rules[{}].name is empty", index));
        }
        if !names.insert(rule.name.clone()) {
            return Err(format!("duplicate Geo rule name: {}", rule.name));
        }
        rule.matches.client_a = normalized_expression(&rule.matches.client_a);
        rule.matches.client_b = normalized_expression(&rule.matches.client_b);
        normalize_unique(
            &mut rule.relays,
            &format!("Geo rule '{}' relays", rule.name),
        )?;
        if rule.relays.is_empty() {
            return Err(format!("Geo rule '{}' has no relays", rule.name));
        }
        for relay in &rule.relays {
            if !relay_set.contains(&relay.to_ascii_lowercase()) {
                return Err(format!(
                    "Geo rule '{}' references relay '{}' which is absent from relay_servers",
                    rule.name, relay
                ));
            }
        }
    }
    crate::geo_relay::validate_config(config)
}

fn normalize_unique(values: &mut Vec<String>, field: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for value in values.iter_mut() {
        *value = value.trim().to_owned();
        if value.is_empty() {
            return Err(format!("{field} contains an empty value"));
        }
        if !seen.insert(value.to_ascii_lowercase()) {
            return Err(format!("{field} contains duplicate value '{value}'"));
        }
    }
    Ok(())
}

fn normalized_expression(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "*".to_owned()
    } else {
        value.to_owned()
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_configuration_means_upstream_behavior() {
        let directory =
            std::env::temp_dir().join(format!("starry-config-empty-{}", std::process::id()));
        let path = directory.join("config.yaml");
        let _ = fs::remove_dir_all(&directory);
        let messages = ensure_artifacts(&path);
        assert!(!messages.is_empty());
        assert!(matches!(load_config(&path), Ok(None)));
        assert!(directory.join("config.example.yaml").is_file());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_geo_relay_reference_rejects_the_entire_config() {
        let raw = r#"
version: 1
relay_servers: [relay-a]
geo:
  enabled: true
  rules:
    - name: bad
      match: { client_a: CN, client_b: JP }
      relays: [relay-b]
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("absent from relay_servers"));
    }

    #[test]
    fn example_configuration_is_valid() {
        assert!(parse_config(EXAMPLE_CONFIG).is_ok());
    }

    #[test]
    fn version_one_keeps_websocket_signal_disabled() {
        let config = parse_config("version: 1\n").unwrap();
        assert!(!config.websocket_signal.enabled);
    }

    #[test]
    fn version_one_rejects_websocket_signal_fields() {
        let err = parse_config("version: 1\nwebsocket_signal:\n  enabled: false\n").unwrap_err();
        assert!(err.contains("version 1 does not allow websocket_signal"));
    }

    #[test]
    fn websocket_signal_requires_exact_relay_coverage() {
        let raw = r#"
version: 2
relay_servers: [relay-a.example.com:21117]
websocket_signal:
  enabled: true
  relay_health:
    endpoints: []
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("cover relay_servers exactly"));
    }

    #[test]
    fn websocket_signal_rejects_insecure_or_wrong_path_endpoints() {
        let raw = r#"
version: 2
relay_servers: [relay-a.example.com:21117]
websocket_signal:
  enabled: true
  relay_health:
    endpoints:
      - relay: relay-a.example.com:21117
        url: ws://relay-a.example.com/not-relay
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("exact wss://.../ws/relay"));
    }
}
