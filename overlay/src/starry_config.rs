use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
};

pub const DEFAULT_CONFIG_PATH: &str = "starry/config.yaml";
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MIN_CONFIG_VERSION: u8 = 1;
pub const CONFIG_VERSION: u8 = 4;
const EXAMPLE_CONFIG: &str = include_str!("starry_config.example.yaml");

static STATE: Lazy<RwLock<ConfigState>> = Lazy::new(|| {
    RwLock::new(ConfigState {
        path: PathBuf::from(DEFAULT_CONFIG_PATH),
        config: None,
        generation: 0,
        source_digest: None,
        effective_digest: None,
        activated_at: None,
        subsystem_acks: Vec::new(),
        last_error: None,
    })
});

struct ConfigState {
    path: PathBuf,
    config: Option<Arc<StarryConfig>>,
    generation: u64,
    source_digest: Option<String>,
    effective_digest: Option<String>,
    activated_at: Option<String>,
    subsystem_acks: Vec<SubsystemAck>,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConfigDiagnostic {
    pub code: String,
    pub pointer: String,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub severity: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostics {
    pub errors: Vec<ConfigDiagnostic>,
}

impl Diagnostics {
    fn single(code: &str, pointer: &str, message: impl Into<String>) -> Self {
        Self {
            errors: vec![ConfigDiagnostic {
                code: code.to_owned(),
                pointer: pointer.to_owned(),
                message: message.into(),
                line: None,
                column: None,
                severity: "error".to_owned(),
            }],
        }
    }

    pub fn summary(&self) -> String {
        self.errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

pub struct ParsedConfig {
    raw: Vec<u8>,
    wire: StarryConfigWire,
}

#[derive(Clone, Debug)]
pub struct ValidatedConfig {
    pub config: StarryConfig,
    pub source_digest: String,
    pub effective_digest: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConfigChange {
    pub pointer: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivationPlan {
    pub base_generation: u64,
    pub candidate_source_digest: String,
    pub candidate_effective_digest: String,
    pub schema_version: u8,
    pub changes: Vec<ConfigChange>,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubsystemAck {
    pub subsystem: String,
    pub accepted: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivationAck {
    pub generation: u64,
    pub schema_version: u8,
    pub source_digest: String,
    pub effective_digest: String,
    pub activated_at: String,
    pub subsystem_acks: Vec<SubsystemAck>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeConfigState {
    pub status: String,
    pub generation: u64,
    pub schema_version: Option<u8>,
    pub source_digest: Option<String>,
    pub effective_digest: Option<String>,
    pub activated_at: Option<String>,
    pub subsystem_acks: Vec<SubsystemAck>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct ActiveConfigSnapshot {
    pub generation: u64,
    pub config: Option<Arc<StarryConfig>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StarryConfig {
    pub version: u8,
    pub relay_servers: Vec<String>,
    pub secure_tcp: SecureTcpConfig,
    pub mmdb: MmdbConfig,
    pub geo: GeoConfig,
    pub websocket_signal: WebSocketSignalConfig,
    pub connection_auth: ConnectionAuthConfig,
    pub relay_quality: RelayQualityConfig,
    pub fast_mode: FastModeConfig,
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
    #[serde(default)]
    connection_auth: Option<ConnectionAuthConfig>,
    #[serde(default)]
    relay_quality: Option<RelayQualityConfig>,
    #[serde(default)]
    fast_mode: Option<FastModeConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecureTcpMode {
    Off,
    Auto,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayEndpointConfig {
    pub relay: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_secret_file: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelayQualityConfig {
    pub enabled: bool,
    pub strategy: RelayQualityStrategyConfig,
    pub legacy_fallback_relays: Vec<String>,
    pub max_candidates: usize,
    pub primary_probe_samples: u32,
    pub primary_accept_score: u32,
    pub primary_max_loss_basis_points: u32,
    pub p2p_probe_grace_ms: u32,
    pub probe_samples: u32,
    pub probe_interval_ms: u32,
    pub probe_timeout_ms: u32,
    pub report_timeout_ms: u32,
    pub max_telemetry_age_seconds: u64,
    pub allocation_ttl_seconds: u64,
    pub cache_ttl_seconds: u64,
    pub max_allocations: usize,
    pub hysteresis_basis_points: u32,
    pub missing_report_penalty_basis_points: u32,
    pub rtt_bad_ms: u32,
    pub jitter_bad_ms: u32,
    pub weights: RelayQualityWeightsConfig,
}

impl Default for RelayQualityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: RelayQualityStrategyConfig::Adaptive,
            legacy_fallback_relays: Vec::new(),
            max_candidates: 3,
            primary_probe_samples: 3,
            primary_accept_score: 8_000,
            primary_max_loss_basis_points: 500,
            p2p_probe_grace_ms: 300,
            probe_samples: 5,
            probe_interval_ms: 50,
            probe_timeout_ms: 1_000,
            report_timeout_ms: 15_000,
            max_telemetry_age_seconds: 180,
            allocation_ttl_seconds: 30,
            cache_ttl_seconds: 300,
            max_allocations: 10_000,
            hysteresis_basis_points: 500,
            missing_report_penalty_basis_points: 1_000,
            rtt_bad_ms: 300,
            jitter_bad_ms: 100,
            weights: RelayQualityWeightsConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayQualityStrategyConfig {
    #[default]
    Adaptive,
    Eager,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelayQualityWeightsConfig {
    pub rtt: u32,
    pub jitter: u32,
    pub loss: u32,
    pub load: u32,
}

impl Default for RelayQualityWeightsConfig {
    fn default() -> Self {
        Self {
            rtt: 4_000,
            jitter: 2_000,
            loss: 2_500,
            load: 1_500,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FastModeConfig {
    pub relay: FastRelayConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FastRelayConfig {
    pub fast_compat_enabled: bool,
    pub authorization_ttl_seconds: u64,
    pub max_bitrate_kbps: u32,
}

impl Default for FastRelayConfig {
    fn default() -> Self {
        Self {
            fast_compat_enabled: false,
            authorization_ttl_seconds: 90,
            max_bitrate_kbps: 50_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionAuthMode {
    Off,
    Audit,
    Enforce,
}

impl Default for ConnectionAuthMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionAuthConfig {
    pub mode: ConnectionAuthMode,
    pub issuer: String,
    pub audience: String,
    pub token_use: String,
    pub required_scope: String,
    pub max_token_bytes: usize,
    pub clock_skew_seconds: u64,
    pub jwks: JwksConfig,
    pub introspection: IntrospectionConfig,
}

impl Default for ConnectionAuthConfig {
    fn default() -> Self {
        Self {
            mode: ConnectionAuthMode::Off,
            issuer: String::new(),
            audience: String::new(),
            token_use: "access".to_owned(),
            required_scope: "connect:initiate".to_owned(),
            max_token_bytes: 8_192,
            clock_skew_seconds: 30,
            jwks: JwksConfig::default(),
            introspection: IntrospectionConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JwksConfig {
    pub file: String,
    pub url: String,
    pub refresh_interval_seconds: u64,
    pub max_stale_seconds: u64,
    pub ca_file: String,
    pub cert_file: String,
    pub key_file: String,
    pub server_name: String,
}

impl Default for JwksConfig {
    fn default() -> Self {
        Self {
            file: String::new(),
            url: String::new(),
            refresh_interval_seconds: 300,
            max_stale_seconds: 3_600,
            ca_file: String::new(),
            cert_file: String::new(),
            key_file: String::new(),
            server_name: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntrospectionConfig {
    pub required: bool,
    pub url: String,
    pub timeout_ms: u64,
    pub positive_cache_seconds: u64,
    pub negative_cache_seconds: u64,
    pub max_cache_entries: usize,
    pub ca_file: String,
    pub cert_file: String,
    pub key_file: String,
    pub server_name: String,
}

impl Default for IntrospectionConfig {
    fn default() -> Self {
        Self {
            required: false,
            url: String::new(),
            timeout_ms: 1_000,
            positive_cache_seconds: 10,
            negative_cache_seconds: 1,
            max_cache_entries: 100_000,
            ca_file: String::new(),
            cert_file: String::new(),
            key_file: String::new(),
            server_name: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeoConfig {
    pub enabled: bool,
    pub rules: Vec<GeoRuleConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeoRuleConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub symmetric: bool,
    #[serde(rename = "match")]
    pub matches: EndpointExpressions,
    pub relays: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    /// True only when a complete, valid document was accepted for activation.
    /// Callers must not reconfigure subsystems when this is false.
    pub accepted: bool,
    pub activation_ack: Option<ActivationAck>,
    pub error: Option<String>,
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
                accepted: false,
                activation_ack: None,
                error: Some(format!("configuration state lock failed: {err}")),
            }
        }
    };
    apply_loaded(path, Vec::new())
}

pub fn snapshot() -> Option<Arc<StarryConfig>> {
    STATE.read().ok()?.config.clone()
}

pub fn active_snapshot() -> ActiveConfigSnapshot {
    match STATE.read() {
        Ok(state) => ActiveConfigSnapshot {
            generation: state.generation,
            config: state.config.clone(),
        },
        Err(_) => ActiveConfigSnapshot {
            generation: 0,
            config: None,
        },
    }
}

pub fn load_candidate() -> Result<ValidatedConfig, Diagnostics> {
    let path = STATE
        .read()
        .map_err(|err| {
            Diagnostics::single(
                "CONFIG_STATE_UNAVAILABLE",
                "",
                format!("configuration state lock failed: {err}"),
            )
        })?
        .path
        .clone();
    let raw = fs::read(&path).map_err(|err| {
        Diagnostics::single(
            "CONFIG_UNREADABLE",
            "",
            format!("cannot read configuration {}: {err}", path.display()),
        )
    })?;
    validate_config(parse_document(&raw)?)
}

pub(crate) fn config_directory() -> PathBuf {
    STATE
        .read()
        .ok()
        .and_then(|state| state.path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn runtime_state() -> RuntimeConfigState {
    match STATE.read() {
        Ok(state) => RuntimeConfigState {
            status: if state.config.is_some() {
                "active".to_owned()
            } else {
                "disabled_no_config".to_owned()
            },
            generation: state.generation,
            schema_version: state.config.as_ref().map(|config| config.version),
            source_digest: state.source_digest.clone(),
            effective_digest: state.effective_digest.clone(),
            activated_at: state.activated_at.clone(),
            subsystem_acks: state.subsystem_acks.clone(),
            last_error: state.last_error.clone(),
        },
        Err(err) => RuntimeConfigState {
            status: "unavailable".to_owned(),
            generation: 0,
            schema_version: None,
            source_digest: None,
            effective_digest: None,
            activated_at: None,
            subsystem_acks: Vec::new(),
            last_error: Some(format!("configuration state lock failed: {err}")),
        },
    }
}

pub fn plan_activation(next: &ValidatedConfig) -> Result<ActivationPlan, Diagnostics> {
    let state = STATE.read().map_err(|err| {
        Diagnostics::single(
            "CONFIG_STATE_UNAVAILABLE",
            "",
            format!("configuration state lock failed: {err}"),
        )
    })?;
    let current = state
        .config
        .as_ref()
        .map(|config| serde_json::to_value(config.as_ref()))
        .transpose()
        .map_err(|err| {
            Diagnostics::single(
                "CONFIG_SERIALIZATION_FAILED",
                "",
                format!("cannot serialize active configuration: {err}"),
            )
        })?;
    let candidate = serde_json::to_value(&next.config).map_err(|err| {
        Diagnostics::single(
            "CONFIG_SERIALIZATION_FAILED",
            "",
            format!("cannot serialize candidate configuration: {err}"),
        )
    })?;
    let mut changes = Vec::new();
    collect_changes("", current.as_ref(), Some(&candidate), &mut changes);
    Ok(ActivationPlan {
        base_generation: state.generation,
        candidate_source_digest: next.source_digest.clone(),
        candidate_effective_digest: next.effective_digest.clone(),
        schema_version: next.config.version,
        changes,
        restart_required: false,
    })
}

pub fn activate(
    next: ValidatedConfig,
    subsystem_acks: Vec<SubsystemAck>,
) -> Result<ActivationAck, Diagnostics> {
    if subsystem_acks.iter().any(|ack| !ack.accepted) {
        return Err(Diagnostics::single(
            "SUBSYSTEM_PREPARE_REJECTED",
            "",
            "one or more subsystems rejected the candidate; active configuration was not changed",
        ));
    }
    let mut state = STATE.write().map_err(|err| {
        Diagnostics::single(
            "CONFIG_STATE_UNAVAILABLE",
            "",
            format!("configuration state lock failed: {err}"),
        )
    })?;
    Ok(activate_state(&mut state, next, subsystem_acks))
}

pub fn activate_if_base_generation(
    next: ValidatedConfig,
    subsystem_acks: Vec<SubsystemAck>,
    expected_base_generation: u64,
) -> Result<ActivationAck, Diagnostics> {
    if subsystem_acks.iter().any(|ack| !ack.accepted) {
        return Err(Diagnostics::single(
            "SUBSYSTEM_PREPARE_REJECTED",
            "",
            "one or more subsystems rejected the candidate; active configuration was not changed",
        ));
    }
    let mut state = STATE.write().map_err(|err| {
        Diagnostics::single(
            "CONFIG_STATE_UNAVAILABLE",
            "",
            format!("configuration state lock failed: {err}"),
        )
    })?;
    if state.generation != expected_base_generation {
        return Err(Diagnostics::single(
            "PLAN_STALE",
            "",
            format!(
                "active generation changed from {expected_base_generation} to {}",
                state.generation
            ),
        ));
    }
    Ok(activate_state(&mut state, next, subsystem_acks))
}

pub fn acknowledge_active(
    expected_generation: u64,
    subsystem_acks: Vec<SubsystemAck>,
) -> Result<ActivationAck, Diagnostics> {
    if subsystem_acks.is_empty() || subsystem_acks.iter().any(|ack| !ack.accepted) {
        return Err(Diagnostics::single(
            "SUBSYSTEM_PREPARE_REJECTED",
            "",
            "every required subsystem must acknowledge the active configuration",
        ));
    }
    let mut state = STATE.write().map_err(|err| {
        Diagnostics::single(
            "CONFIG_STATE_UNAVAILABLE",
            "",
            format!("configuration state lock failed: {err}"),
        )
    })?;
    if state.generation != expected_generation {
        return Err(Diagnostics::single(
            "PLAN_STALE",
            "",
            format!(
                "active generation changed from {expected_generation} to {}",
                state.generation
            ),
        ));
    }
    let config = state.config.as_ref().ok_or_else(|| {
        Diagnostics::single(
            "STARRY_NOT_READY",
            "",
            "cannot acknowledge subsystems without an active configuration",
        )
    })?;
    let schema_version = config.version;
    let source_digest = state.source_digest.clone().unwrap_or_default();
    let effective_digest = state.effective_digest.clone().unwrap_or_default();
    let activated_at = state
        .activated_at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    state.subsystem_acks = subsystem_acks.clone();
    state.last_error = None;
    Ok(ActivationAck {
        generation: state.generation,
        schema_version,
        source_digest,
        effective_digest,
        activated_at,
        subsystem_acks,
    })
}

fn activate_state(
    state: &mut ConfigState,
    next: ValidatedConfig,
    mut subsystem_acks: Vec<SubsystemAck>,
) -> ActivationAck {
    if subsystem_acks.is_empty() {
        subsystem_acks.push(SubsystemAck {
            subsystem: "config_core".to_owned(),
            accepted: true,
            detail: "parsed and validated".to_owned(),
        });
    }
    state.generation = state.generation.saturating_add(1);
    let activated_at = chrono::Utc::now().to_rfc3339();
    state.config = Some(Arc::new(next.config.clone()));
    state.source_digest = Some(next.source_digest.clone());
    state.effective_digest = Some(next.effective_digest.clone());
    state.activated_at = Some(activated_at.clone());
    state.subsystem_acks = subsystem_acks.clone();
    state.last_error = None;
    ActivationAck {
        generation: state.generation,
        schema_version: next.config.version,
        source_digest: next.source_digest,
        effective_digest: next.effective_digest,
        activated_at,
        subsystem_acks,
    }
}

fn apply_loaded(path: PathBuf, mut artifact_messages: Vec<String>) -> ReloadOutcome {
    let loaded = load_config(&path);
    let mut accepted = false;
    let mut activation_ack = None;
    let load_message;
    let mut relay_servers = None;
    let mut error = None;

    match STATE.write() {
        Ok(mut state) => {
            state.path = path.clone();
            match loaded {
                Ok(Some(next)) => {
                    let relay_count = next.config.relay_servers.len();
                    let rule_count = next.config.geo.rules.len();
                    relay_servers = joined_relays(&next.config);
                    activation_ack = Some(activate_state(&mut state, next, Vec::new()));
                    accepted = true;
                    load_message = format!(
                        "Starry config loaded from {}: {relay_count} relays, {rule_count} Geo rules",
                        path.display()
                    );
                }
                Ok(None) => {
                    if let Some(active) = state.config.as_ref() {
                        relay_servers = joined_relays(active);
                        load_message = format!(
                            "Starry config {} is empty; rejected reload and retained last-known-good generation {}",
                            path.display(),
                            state.generation
                        );
                        state.last_error = Some("CONFIG_EMPTY".to_owned());
                    } else {
                        load_message = format!(
                            "Starry config {} is empty; using upstream behavior",
                            path.display()
                        );
                        state.last_error = None;
                    }
                }
                Err(err) => {
                    let retained = state.config.is_some();
                    if let Some(active) = state.config.as_ref() {
                        relay_servers = joined_relays(active);
                    }
                    load_message = if retained {
                        format!(
                            "Starry config {} is invalid; rejected reload and retained last-known-good generation {}: {err}",
                            path.display(),
                            state.generation
                        )
                    } else {
                        format!(
                            "Starry config {} is invalid; using upstream behavior: {err}",
                            path.display()
                        )
                    };
                    state.last_error = Some(err.clone());
                    error = Some(err);
                }
            }
        }
        Err(err) => {
            load_message = format!("Starry config state lock failed: {err}");
            error = Some(format!("configuration state lock failed: {err}"));
        }
    }
    artifact_messages.push(load_message);

    ReloadOutcome {
        message: artifact_messages.join("; "),
        relay_servers,
        accepted,
        activation_ack,
        error,
    }
}

fn joined_relays(config: &StarryConfig) -> Option<String> {
    if config.relay_servers.is_empty() {
        None
    } else {
        Some(config.relay_servers.join(","))
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

fn load_config(path: &Path) -> Result<Option<ValidatedConfig>, String> {
    let raw = fs::read(path).map_err(|err| format!("cannot read configuration: {err}"))?;
    if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(None);
    }
    let parsed = parse_document(&raw).map_err(|errors| errors.summary())?;
    validate_config(parsed)
        .map(Some)
        .map_err(|errors| errors.summary())
}

fn parse_config(raw: &str) -> Result<StarryConfig, String> {
    let parsed = parse_document(raw.as_bytes()).map_err(|errors| errors.summary())?;
    validate_config(parsed)
        .map(|validated| validated.config)
        .map_err(|errors| errors.summary())
}

pub fn parse_document(raw: &[u8]) -> Result<ParsedConfig, Diagnostics> {
    if raw.len() > MAX_CONFIG_BYTES {
        return Err(Diagnostics::single(
            "CONFIG_TOO_LARGE",
            "",
            format!(
                "configuration is {} bytes; maximum is {MAX_CONFIG_BYTES}",
                raw.len()
            ),
        ));
    }
    if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err(Diagnostics::single(
            "CONFIG_EMPTY",
            "",
            "configuration document is empty",
        ));
    }
    let wire: StarryConfigWire = serde_yml::from_slice(raw).map_err(|err| {
        let mut diagnostic = ConfigDiagnostic {
            code: "YAML_INVALID".to_owned(),
            pointer: String::new(),
            message: format!("invalid YAML: {err}"),
            line: None,
            column: None,
            severity: "error".to_owned(),
        };
        if let Some(location) = err.location() {
            diagnostic.line = Some(location.line());
            diagnostic.column = Some(location.column());
        }
        Diagnostics {
            errors: vec![diagnostic],
        }
    })?;
    Ok(ParsedConfig {
        raw: raw.to_vec(),
        wire,
    })
}

pub fn validate_config(parsed: ParsedConfig) -> Result<ValidatedConfig, Diagnostics> {
    let ParsedConfig { raw, wire } = parsed;
    if !(MIN_CONFIG_VERSION..=CONFIG_VERSION).contains(&wire.version) {
        return Err(Diagnostics::single(
            "SCHEMA_UNSUPPORTED",
            "/version",
            format!(
                "unsupported version {}; expected 1, 2, 3, or {CONFIG_VERSION}",
                wire.version
            ),
        ));
    }
    if wire.version == 1 && wire.websocket_signal.is_some() {
        return Err(Diagnostics::single(
            "FIELD_REQUIRES_SCHEMA_V2",
            "/websocket_signal",
            "version 1 does not allow websocket_signal; upgrade the document to version 2",
        ));
    }
    if wire.version < 3 && wire.connection_auth.is_some() {
        return Err(Diagnostics::single(
            "FIELD_REQUIRES_SCHEMA_V3",
            "/connection_auth",
            format!(
                "version {} does not allow connection_auth; upgrade the document to version 3",
                wire.version
            ),
        ));
    }
    if wire.version < 4 && wire.relay_quality.is_some() {
        return Err(Diagnostics::single(
            "FIELD_REQUIRES_SCHEMA_V4",
            "/relay_quality",
            format!(
                "version {} does not allow relay_quality; upgrade the document to version 4",
                wire.version
            ),
        ));
    }
    if wire.version < 4 && wire.fast_mode.is_some() {
        return Err(Diagnostics::single(
            "FIELD_REQUIRES_SCHEMA_V4",
            "/fast_mode",
            format!(
                "version {} does not allow fast_mode; upgrade the document to version 4",
                wire.version
            ),
        ));
    }
    let config = StarryConfig {
        version: wire.version,
        relay_servers: wire.relay_servers,
        secure_tcp: wire.secure_tcp,
        mmdb: wire.mmdb,
        geo: wire.geo,
        websocket_signal: wire.websocket_signal.unwrap_or_default(),
        connection_auth: wire.connection_auth.unwrap_or_default(),
        relay_quality: wire.relay_quality.unwrap_or_default(),
        fast_mode: wire.fast_mode.unwrap_or_default(),
    };
    let config = validate(config)
        .map_err(|err| Diagnostics::single("CONFIG_INVALID", diagnostic_pointer(&err), err))?;
    let effective = serde_json::to_vec(&config).map_err(|err| {
        Diagnostics::single(
            "CONFIG_SERIALIZATION_FAILED",
            "",
            format!("cannot serialize normalized configuration: {err}"),
        )
    })?;
    Ok(ValidatedConfig {
        config,
        source_digest: sha256_digest(&raw),
        effective_digest: sha256_digest(&effective),
    })
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
    validate_connection_auth(&mut config.connection_auth)?;
    validate_relay_quality(
        &mut config.relay_quality,
        &config.relay_servers,
        &config.websocket_signal.relay_health,
    )?;
    validate_fast_mode(&config)?;
    Ok(config)
}

fn validate_fast_mode(config: &StarryConfig) -> Result<(), String> {
    let relay = &config.fast_mode.relay;
    if !(30..=300).contains(&relay.authorization_ttl_seconds) {
        return Err(
            "fast_mode.relay.authorization_ttl_seconds must be between 30 and 300".to_owned(),
        );
    }
    if !(1_000..=200_000).contains(&relay.max_bitrate_kbps) {
        return Err("fast_mode.relay.max_bitrate_kbps must be between 1000 and 200000".to_owned());
    }
    if relay.fast_compat_enabled && !config.relay_quality.enabled {
        return Err(
            "fast_mode.relay.fast_compat_enabled requires relay_quality.enabled".to_owned(),
        );
    }
    if relay.fast_compat_enabled && config.connection_auth.mode == ConnectionAuthMode::Off {
        return Err(
            "fast_mode.relay.fast_compat_enabled requires connection_auth.mode audit or enforce"
                .to_owned(),
        );
    }
    if relay.fast_compat_enabled
        && config.secure_tcp.mode == SecureTcpMode::Off
        && !config.websocket_signal.enabled
    {
        return Err(
            "fast_mode.relay.fast_compat_enabled requires Secure TCP or WebSocket signalling"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_relay_quality(
    config: &mut RelayQualityConfig,
    relay_servers: &[String],
    relay_health: &RelayHealthConfig,
) -> Result<(), String> {
    normalize_unique(
        &mut config.legacy_fallback_relays,
        "relay_quality.legacy_fallback_relays",
    )?;
    if !(1..=5).contains(&config.max_candidates) {
        return Err("relay_quality.max_candidates must be between 1 and 5".to_owned());
    }
    if !(1..=20).contains(&config.primary_probe_samples)
        || config.primary_probe_samples > config.probe_samples
    {
        return Err(
            "relay_quality.primary_probe_samples must be between 1 and probe_samples".to_owned(),
        );
    }
    if !(1..=10_000).contains(&config.primary_accept_score) {
        return Err("relay_quality.primary_accept_score must be between 1 and 10000".to_owned());
    }
    if config.primary_max_loss_basis_points > 10_000 {
        return Err("relay_quality.primary_max_loss_basis_points must not exceed 10000".to_owned());
    }
    if config.p2p_probe_grace_ms > 5_000 {
        return Err("relay_quality.p2p_probe_grace_ms must not exceed 5000".to_owned());
    }
    if !(3..=20).contains(&config.probe_samples) {
        return Err("relay_quality.probe_samples must be between 3 and 20".to_owned());
    }
    if !(20..=2_000).contains(&config.probe_interval_ms) {
        return Err("relay_quality.probe_interval_ms must be between 20 and 2000".to_owned());
    }
    if !(100..=5_000).contains(&config.probe_timeout_ms) {
        return Err("relay_quality.probe_timeout_ms must be between 100 and 5000".to_owned());
    }
    if !(1_000..=60_000).contains(&config.report_timeout_ms) {
        return Err("relay_quality.report_timeout_ms must be between 1000 and 60000".to_owned());
    }
    if !(5..=3_600).contains(&config.max_telemetry_age_seconds) {
        return Err(
            "relay_quality.max_telemetry_age_seconds must be between 5 and 3600".to_owned(),
        );
    }
    if !(5..=300).contains(&config.allocation_ttl_seconds) {
        return Err("relay_quality.allocation_ttl_seconds must be between 5 and 300".to_owned());
    }
    if !(30..=86_400).contains(&config.cache_ttl_seconds) {
        return Err("relay_quality.cache_ttl_seconds must be between 30 and 86400".to_owned());
    }
    if !(100..=1_000_000).contains(&config.max_allocations) {
        return Err("relay_quality.max_allocations must be between 100 and 1000000".to_owned());
    }
    if config.hysteresis_basis_points > 5_000 {
        return Err("relay_quality.hysteresis_basis_points must not exceed 5000".to_owned());
    }
    if config.missing_report_penalty_basis_points > 10_000 {
        return Err(
            "relay_quality.missing_report_penalty_basis_points must not exceed 10000".to_owned(),
        );
    }
    if !(10..=10_000).contains(&config.rtt_bad_ms) {
        return Err("relay_quality.rtt_bad_ms must be between 10 and 10000".to_owned());
    }
    if !(1..=5_000).contains(&config.jitter_bad_ms) {
        return Err("relay_quality.jitter_bad_ms must be between 1 and 5000".to_owned());
    }
    let weights = &config.weights;
    let total = weights
        .rtt
        .saturating_add(weights.jitter)
        .saturating_add(weights.loss)
        .saturating_add(weights.load);
    if total != 10_000 || [weights.rtt, weights.jitter, weights.loss, weights.load].contains(&0) {
        return Err(
            "relay_quality.weights must all be positive and sum to 10000 basis points".to_owned(),
        );
    }
    if config.enabled && relay_servers.len() < 2 {
        return Err(
            "relay_quality.enabled requires at least two configured Relay servers".to_owned(),
        );
    }
    if config.enabled && config.max_candidates < 2 {
        return Err("relay_quality.enabled requires max_candidates to be at least 2".to_owned());
    }
    let configured: HashSet<String> = relay_servers
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let legacy: HashSet<String> = config
        .legacy_fallback_relays
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    if let Some(unknown) = config
        .legacy_fallback_relays
        .iter()
        .find(|relay| !configured.contains(&relay.to_ascii_lowercase()))
    {
        return Err(format!(
            "relay_quality.legacy_fallback_relays references '{unknown}' which is absent from relay_servers"
        ));
    }
    if config.enabled {
        let quality_relays = relay_servers
            .iter()
            .filter(|relay| !legacy.contains(&relay.to_ascii_lowercase()))
            .collect::<Vec<_>>();
        if quality_relays.len() < 2 {
            return Err(
                "relay_quality.enabled requires at least two non-legacy quality Relay servers"
                    .to_owned(),
            );
        }
        let telemetry_relays: HashSet<String> = relay_health
            .endpoints
            .iter()
            .map(|endpoint| endpoint.relay.to_ascii_lowercase())
            .collect();
        let missing = quality_relays
            .iter()
            .filter(|relay| !telemetry_relays.contains(&relay.to_ascii_lowercase()))
            .map(|relay| relay.as_str())
            .collect::<Vec<_>>();
        let unknown = relay_health
            .endpoints
            .iter()
            .filter(|endpoint| !configured.contains(&endpoint.relay.to_ascii_lowercase()))
            .map(|endpoint| endpoint.relay.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() || !unknown.is_empty() {
            return Err(format!(
                "relay_quality.enabled requires a unique health/telemetry endpoint for every non-legacy Relay; missing={missing:?}, unknown={unknown:?}"
            ));
        }
        let insecure = relay_health
            .endpoints
            .iter()
            .filter(|endpoint| !legacy.contains(&endpoint.relay.to_ascii_lowercase()))
            .filter(|endpoint| {
                url::Url::parse(&endpoint.url)
                    .ok()
                    .map(|url| url.path() != "/ws/telemetry")
                    .unwrap_or(true)
                    || endpoint.telemetry_secret_file.is_none()
            })
            .map(|endpoint| endpoint.relay.as_str())
            .collect::<Vec<_>>();
        if !insecure.is_empty() {
            return Err(format!(
                "relay_quality.enabled requires authenticated /ws/telemetry endpoints with telemetry_secret_file for every non-legacy Relay; invalid={insecure:?}"
            ));
        }
        let minimum_freshness = relay_health
            .interval_seconds
            .saturating_add(relay_health.timeout_ms.saturating_add(999) / 1_000);
        if config.max_telemetry_age_seconds < minimum_freshness {
            return Err(format!(
                "relay_quality.max_telemetry_age_seconds must be at least relay health interval plus timeout ({minimum_freshness} seconds)"
            ));
        }
        // Candidates within one stage are concurrent, while samples for a
        // candidate are ordered. The initial target report precedes the
        // controller report on the rendezvous path. Adaptive mode therefore
        // needs two primary windows plus one concurrent expanded window and a
        // bounded signalling margin. Eager mode retains two full endpoint
        // windows. This is a proof that the configured total deadline is
        // achievable without assuming infinite waits.
        let probe_window_ms = |samples: u32| {
            u64::from(samples)
                .saturating_mul(u64::from(config.probe_timeout_ms))
                .saturating_add(
                    u64::from(samples.saturating_sub(1))
                        .saturating_mul(u64::from(config.probe_interval_ms)),
                )
        };
        let required_report_ms = match config.strategy {
            RelayQualityStrategyConfig::Adaptive => u64::from(config.p2p_probe_grace_ms)
                .saturating_add(probe_window_ms(config.primary_probe_samples))
                .saturating_mul(2)
                .saturating_add(probe_window_ms(config.probe_samples))
                .saturating_add(1_000),
            RelayQualityStrategyConfig::Eager => {
                probe_window_ms(config.probe_samples).saturating_mul(2)
            }
        };
        if u64::from(config.report_timeout_ms) < required_report_ms {
            return Err(format!(
                "relay_quality report deadline is not achievable for {:?}: report_timeout_ms={} but staged probe windows require at least {required_report_ms}ms",
                config.strategy,
                config.report_timeout_ms
            ));
        }
        if config.allocation_ttl_seconds.saturating_mul(1_000)
            <= u64::from(config.report_timeout_ms)
        {
            return Err(
                "relay_quality.allocation_ttl_seconds must exceed report_timeout_ms; TTL is cleanup only"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

pub fn effective_connection_auth_mode(
    configured: ConnectionAuthMode,
    must_login: bool,
) -> ConnectionAuthMode {
    if must_login {
        ConnectionAuthMode::Enforce
    } else {
        configured
    }
}

fn validate_connection_auth(config: &mut ConnectionAuthConfig) -> Result<(), String> {
    for (field, value) in [
        ("issuer", &mut config.issuer),
        ("audience", &mut config.audience),
        ("token_use", &mut config.token_use),
        ("required_scope", &mut config.required_scope),
        ("jwks.file", &mut config.jwks.file),
        ("jwks.url", &mut config.jwks.url),
        ("jwks.ca_file", &mut config.jwks.ca_file),
        ("jwks.cert_file", &mut config.jwks.cert_file),
        ("jwks.key_file", &mut config.jwks.key_file),
        ("jwks.server_name", &mut config.jwks.server_name),
        ("introspection.url", &mut config.introspection.url),
        ("introspection.ca_file", &mut config.introspection.ca_file),
        (
            "introspection.cert_file",
            &mut config.introspection.cert_file,
        ),
        ("introspection.key_file", &mut config.introspection.key_file),
        (
            "introspection.server_name",
            &mut config.introspection.server_name,
        ),
    ] {
        *value = value.trim().to_owned();
        if value.chars().any(|ch| matches!(ch, '\0' | '\n' | '\r')) {
            return Err(format!(
                "connection_auth.{field} contains a forbidden character"
            ));
        }
    }

    if !(128..=8_192).contains(&config.max_token_bytes) {
        return Err("connection_auth.max_token_bytes must be between 128 and 8192".to_owned());
    }
    if config.clock_skew_seconds > 300 {
        return Err("connection_auth.clock_skew_seconds must not exceed 300".to_owned());
    }
    if !(30..=86_400).contains(&config.jwks.refresh_interval_seconds) {
        return Err(
            "connection_auth.jwks.refresh_interval_seconds must be between 30 and 86400".to_owned(),
        );
    }
    if config.jwks.max_stale_seconds < config.jwks.refresh_interval_seconds
        || config.jwks.max_stale_seconds > 604_800
    {
        return Err("connection_auth.jwks.max_stale_seconds must be at least refresh_interval_seconds and at most 604800".to_owned());
    }
    if !config.jwks.url.is_empty() {
        validate_https_url(&config.jwks.url, "connection_auth.jwks.url")?;
        validate_mtls_endpoint(
            &config.jwks.url,
            &config.jwks.ca_file,
            &config.jwks.cert_file,
            &config.jwks.key_file,
            &config.jwks.server_name,
            "connection_auth.jwks",
        )?;
    } else if !config.jwks.ca_file.is_empty()
        || !config.jwks.cert_file.is_empty()
        || !config.jwks.key_file.is_empty()
        || !config.jwks.server_name.is_empty()
    {
        return Err("connection_auth.jwks TLS fields require a jwks.url".to_owned());
    }

    let introspection = &config.introspection;
    if !(100..=10_000).contains(&introspection.timeout_ms) {
        return Err(
            "connection_auth.introspection.timeout_ms must be between 100 and 10000".to_owned(),
        );
    }
    if !(1..=60).contains(&introspection.positive_cache_seconds) {
        return Err(
            "connection_auth.introspection.positive_cache_seconds must be between 1 and 60"
                .to_owned(),
        );
    }
    if introspection.negative_cache_seconds > 1 {
        return Err(
            "connection_auth.introspection.negative_cache_seconds must not exceed 1".to_owned(),
        );
    }
    if !(1..=1_000_000).contains(&introspection.max_cache_entries) {
        return Err(
            "connection_auth.introspection.max_cache_entries must be between 1 and 1000000"
                .to_owned(),
        );
    }

    if config.mode != ConnectionAuthMode::Off {
        if config.issuer.is_empty()
            || config.audience.is_empty()
            || config.token_use.is_empty()
            || config.required_scope.is_empty()
        {
            return Err("connection_auth issuer, audience, token_use, and required_scope are required when mode is audit or enforce".to_owned());
        }
        validate_https_url(&config.issuer, "connection_auth.issuer")?;
        if config.jwks.file.is_empty() && config.jwks.url.is_empty() {
            return Err("connection_auth.jwks.file or connection_auth.jwks.url is required when authentication is enabled".to_owned());
        }
    }
    if config.mode == ConnectionAuthMode::Enforce && config.jwks.file.is_empty() {
        return Err("connection_auth.jwks.file is required in enforce mode so an initial Ed25519 keyset can be loaded fail-closed".to_owned());
    }

    if introspection.required && introspection.url.is_empty() {
        return Err(
            "connection_auth.introspection.url is required when introspection.required is true"
                .to_owned(),
        );
    }
    if !introspection.url.is_empty() {
        validate_https_url(&introspection.url, "connection_auth.introspection.url")?;
        validate_mtls_endpoint(
            &introspection.url,
            &introspection.ca_file,
            &introspection.cert_file,
            &introspection.key_file,
            &introspection.server_name,
            "connection_auth.introspection",
        )?;
    } else if !introspection.ca_file.is_empty()
        || !introspection.cert_file.is_empty()
        || !introspection.key_file.is_empty()
        || !introspection.server_name.is_empty()
    {
        return Err(
            "connection_auth.introspection TLS fields require an introspection.url".to_owned(),
        );
    }
    Ok(())
}

fn validate_mtls_endpoint(
    endpoint: &str,
    ca_file: &str,
    cert_file: &str,
    key_file: &str,
    server_name: &str,
    field: &str,
) -> Result<(), String> {
    for (name, value) in [
        ("ca_file", ca_file),
        ("cert_file", cert_file),
        ("key_file", key_file),
        ("server_name", server_name),
    ] {
        if value.is_empty() {
            return Err(format!(
                "{field}.{name} is required whenever its URL is configured"
            ));
        }
    }
    let expected = url::Host::parse(server_name)
        .map_err(|err| format!("{field}.server_name is invalid: {err}"))?;
    if !matches!(expected, url::Host::Domain(_)) {
        return Err(format!("{field}.server_name must be a DNS hostname"));
    }
    let parsed =
        url::Url::parse(endpoint).map_err(|err| format!("{field}.url is invalid: {err}"))?;
    if parsed.host_str() != Some(server_name) {
        return Err(format!(
            "{field}.url host must equal configured server_name"
        ));
    }
    Ok(())
}

fn validate_https_url(value: &str, field: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|err| format!("{field} is invalid: {err}"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(format!(
            "{field} must be an HTTPS URL without credentials or fragment"
        ));
    }
    Ok(())
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
    if health.endpoints.len() > 256 {
        return Err(
            "websocket_signal.relay_health.endpoints must not contain more than 256 Relays"
                .to_owned(),
        );
    }

    let mut relays = HashSet::new();
    let mut urls = HashSet::new();
    for (index, endpoint) in health.endpoints.iter_mut().enumerate() {
        endpoint.relay = endpoint.relay.trim().to_owned();
        endpoint.url = endpoint.url.trim().to_owned();
        endpoint.telemetry_secret_file = endpoint
            .telemetry_secret_file
            .take()
            .map(|path| path.trim().to_owned())
            .filter(|path| !path.is_empty());
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
        let telemetry_endpoint = parsed.path() == "/ws/telemetry";
        if parsed.scheme() != "wss"
            || !matches!(parsed.host(), Some(url::Host::Domain(_)))
            || !matches!(parsed.path(), "/ws/relay" | "/ws/telemetry")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(format!(
                "websocket_signal.relay_health endpoint '{}' must use a hostname and exact wss://.../ws/relay or wss://.../ws/telemetry URL without credentials, query, or fragment",
                endpoint.relay
            ));
        }
        match (&endpoint.telemetry_secret_file, telemetry_endpoint) {
            (Some(path), true) if std::path::Path::new(path).is_absolute() => {}
            (Some(_), true) => {
                return Err(format!(
                    "websocket_signal.relay_health endpoint '{}' telemetry_secret_file must be an absolute path",
                    endpoint.relay
                ));
            }
            (None, true) => {
                return Err(format!(
                    "websocket_signal.relay_health endpoint '{}' /ws/telemetry requires telemetry_secret_file",
                    endpoint.relay
                ));
            }
            (Some(_), false) => {
                return Err(format!(
                    "websocket_signal.relay_health endpoint '{}' may set telemetry_secret_file only with /ws/telemetry",
                    endpoint.relay
                ));
            }
            (None, false) => {}
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
        let path = Path::new(&database.path);
        let mut components = path.components();
        if path.is_absolute()
            || !matches!(components.next(), Some(Component::Normal(root)) if root == "mmdb")
            || components.any(|component| !matches!(component, Component::Normal(_)))
            || path.extension().and_then(|value| value.to_str()) != Some("mmdb")
        {
            return Err(format!(
                "mmdb.{label}.path must be a relative mmdb/*.mmdb path without traversal"
            ));
        }
        if !database.url.is_empty() && !database.url.starts_with("https://") {
            return Err(format!("mmdb.{label}.url must use https://"));
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

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn diagnostic_pointer(message: &str) -> &'static str {
    if message.starts_with("fast_mode.") {
        "/fast_mode"
    } else if message.starts_with("relay_quality.") {
        "/relay_quality"
    } else if message.starts_with("connection_auth.") {
        "/connection_auth"
    } else if message.starts_with("websocket_signal.") {
        "/websocket_signal"
    } else if message.starts_with("secure_tcp.") {
        "/secure_tcp"
    } else if message.starts_with("mmdb.") {
        "/mmdb"
    } else if message.starts_with("geo.")
        || message.starts_with("Geo rule")
        || message.starts_with("duplicate Geo")
    {
        "/geo"
    } else if message.starts_with("relay_servers") {
        "/relay_servers"
    } else if message.starts_with("unsupported version") {
        "/version"
    } else {
        ""
    }
}

fn collect_changes(
    pointer: &str,
    current: Option<&serde_json::Value>,
    candidate: Option<&serde_json::Value>,
    changes: &mut Vec<ConfigChange>,
) {
    match (current, candidate) {
        (Some(serde_json::Value::Object(before)), Some(serde_json::Value::Object(after))) => {
            let keys: BTreeSet<&str> = before
                .keys()
                .chain(after.keys())
                .map(String::as_str)
                .collect();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child = format!("{pointer}/{escaped}");
                collect_changes(&child, before.get(key), after.get(key), changes);
            }
        }
        (Some(before), Some(after)) if before == after => {}
        (None, Some(_)) => changes.push(ConfigChange {
            pointer: pointer.to_owned(),
            kind: "add".to_owned(),
        }),
        (Some(_), None) => changes.push(ConfigChange {
            pointer: pointer.to_owned(),
            kind: "remove".to_owned(),
        }),
        (Some(_), Some(_)) => changes.push(ConfigChange {
            pointer: pointer.to_owned(),
            kind: "replace".to_owned(),
        }),
        (None, None) => {}
    }
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
    use std::sync::Mutex;

    static STATE_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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

    #[test]
    fn telemetry_endpoint_requires_an_absolute_secret_file_and_public_health_forbids_one() {
        let missing = r#"
version: 4
relay_servers: [relay-a.example.com:21117]
websocket_signal:
  relay_health:
    endpoints:
      - relay: relay-a.example.com:21117
        url: wss://relay-a.example.com/ws/telemetry
"#;
        let err = parse_config(missing).unwrap_err();
        assert!(err.contains("requires telemetry_secret_file"));

        let relative = missing.replace(
            "        url: wss://relay-a.example.com/ws/telemetry\n",
            "        url: wss://relay-a.example.com/ws/telemetry\n        telemetry_secret_file: secrets/relay\n",
        );
        let err = parse_config(&relative).unwrap_err();
        assert!(err.contains("must be an absolute path"));

        let public_with_secret = missing.replace(
            "        url: wss://relay-a.example.com/ws/telemetry\n",
            "        url: wss://relay-a.example.com/ws/relay\n        telemetry_secret_file: /run/secrets/relay\n",
        );
        let err = parse_config(&public_with_secret).unwrap_err();
        assert!(err.contains("only with /ws/telemetry"));
    }

    #[test]
    fn invalid_reload_preserves_the_last_known_good_configuration() {
        let _guard = STATE_TEST_LOCK.lock().unwrap();
        let directory =
            std::env::temp_dir().join(format!("starry-config-lkg-invalid-{}", std::process::id()));
        let path = directory.join("config.yaml");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "version: 1\nrelay_servers: [relay-a]\n").unwrap();

        let startup = initialize(path.to_str().unwrap());
        assert_eq!(startup.relay_servers.as_deref(), Some("relay-a"));
        let before = runtime_state();
        fs::write(&path, "version: 1\nunknown: true\n").unwrap();

        let rejected = reload();
        assert!(!rejected.accepted);
        let active = snapshot().expect("the valid startup configuration must remain active");
        assert_eq!(active.relay_servers, ["relay-a"]);
        let after = runtime_state();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.source_digest, before.source_digest);
        assert_eq!(after.effective_digest, before.effective_digest);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn impossible_relay_quality_reload_preserves_generation_and_last_known_good() {
        let _guard = STATE_TEST_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "starry-config-quality-reload-{}",
            std::process::id()
        ));
        let path = directory.join("config.yaml");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let valid = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
"#;
        fs::write(&path, valid).unwrap();
        initialize(path.to_str().unwrap());
        let before = runtime_state();

        let impossible = valid.replace(
            "relay_quality:\n  enabled: true\n",
            "relay_quality:\n  enabled: true\n  probe_timeout_ms: 1000\n  report_timeout_ms: 1000\n",
        );
        fs::write(&path, impossible).unwrap();
        let rejected = reload();
        assert!(!rejected.accepted);
        assert!(rejected.message.contains("deadline is not achievable"));
        let after = runtime_state();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.effective_digest, before.effective_digest);
        assert!(snapshot().unwrap().relay_quality.enabled);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn empty_reload_preserves_the_last_known_good_configuration() {
        let _guard = STATE_TEST_LOCK.lock().unwrap();
        let directory =
            std::env::temp_dir().join(format!("starry-config-lkg-empty-{}", std::process::id()));
        let path = directory.join("config.yaml");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "version: 1\nrelay_servers: [relay-b]\n").unwrap();

        let startup = initialize(path.to_str().unwrap());
        assert_eq!(startup.relay_servers.as_deref(), Some("relay-b"));
        let before = runtime_state();
        fs::write(&path, "\n").unwrap();

        let rejected = reload();
        assert!(!rejected.accepted);
        let active = snapshot().expect("the valid startup configuration must remain active");
        assert_eq!(active.relay_servers, ["relay-b"]);
        let after = runtime_state();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.source_digest, before.source_digest);
        assert_eq!(after.effective_digest, before.effective_digest);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn schema_v3_defaults_connection_auth_to_off() {
        let config = parse_config("version: 3\n").unwrap();
        assert_eq!(config.connection_auth.mode, ConnectionAuthMode::Off);
    }

    #[test]
    fn older_schemas_reject_connection_auth() {
        let err = parse_config("version: 2\nconnection_auth:\n  mode: off\n").unwrap_err();
        assert!(err.contains("upgrade the document to version 3"));
    }

    #[test]
    fn schema_v4_enables_bounded_relay_quality_configuration() {
        let raw = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
  max_candidates: 2
  weights: { rtt: 4000, jitter: 2000, loss: 2500, load: 1500 }
"#;
        let config = parse_config(raw).unwrap();
        assert!(config.relay_quality.enabled);
        assert_eq!(
            config.relay_quality.strategy,
            RelayQualityStrategyConfig::Adaptive
        );
        assert_eq!(config.relay_quality.max_candidates, 2);
        assert_eq!(config.relay_quality.primary_probe_samples, 3);
        assert_eq!(config.relay_quality.primary_accept_score, 8_000);
        assert_eq!(config.relay_quality.weights.load, 1_500);
    }

    #[test]
    fn relay_quality_requires_telemetry_coverage_and_explicit_legacy_fallbacks() {
        let missing = r#"
version: 4
relay_servers: [legacy-a:21117, relay-a:21117, relay-b:21117]
relay_quality:
  enabled: true
"#;
        let err = parse_config(missing).unwrap_err();
        assert!(err.contains("health/telemetry endpoint"));

        let valid = r#"
version: 4
relay_servers: [legacy-a:21117, relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
  legacy_fallback_relays: [legacy-a:21117]
"#;
        let parsed = parse_config(valid).unwrap();
        assert_eq!(
            parsed.relay_quality.legacy_fallback_relays,
            ["legacy-a:21117"]
        );

        let duplicate_url = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://shared.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://shared.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
"#;
        let err = parse_config(duplicate_url).unwrap_err();
        assert!(err.contains("duplicate URL"));
    }

    #[test]
    fn relay_quality_rejects_unachievable_probe_and_report_deadlines() {
        let raw = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
  probe_samples: 5
  probe_interval_ms: 50
  probe_timeout_ms: 1000
  report_timeout_ms: 10000
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("deadline is not achievable"));
        assert!(err.contains("13000ms"));
    }

    #[test]
    fn eager_deadline_keeps_v1_all_candidate_compatibility() {
        let raw = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
  strategy: eager
  probe_samples: 5
  probe_interval_ms: 50
  probe_timeout_ms: 1000
  report_timeout_ms: 10400
"#;
        let config = parse_config(raw).unwrap();
        assert_eq!(
            config.relay_quality.strategy,
            RelayQualityStrategyConfig::Eager
        );
        assert_eq!(config.relay_quality.report_timeout_ms, 10_400);

        let impossible_primary = raw
            .replace("strategy: eager", "strategy: adaptive")
            .replace(
                "probe_samples: 5",
                "probe_samples: 3\n  primary_probe_samples: 4",
            );
        let error = parse_config(&impossible_primary).unwrap_err();
        assert!(error.contains("primary_probe_samples"));
    }

    #[test]
    fn relay_quality_rejects_telemetry_age_shorter_than_health_cycle() {
        let raw = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    interval_seconds: 60
    timeout_ms: 5000
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
relay_quality:
  enabled: true
  max_telemetry_age_seconds: 64
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("at least relay health interval plus timeout (65 seconds)"));
    }

    #[test]
    fn older_schemas_reject_relay_quality_and_v4_rejects_bad_weights() {
        let old = parse_config("version: 3\nrelay_quality:\n  enabled: false\n").unwrap_err();
        assert!(old.contains("upgrade the document to version 4"));

        let invalid = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
relay_quality:
  enabled: true
  weights: { rtt: 5000, jitter: 2000, loss: 2500, load: 1500 }
"#;
        let err = parse_config(invalid).unwrap_err();
        assert!(err.contains("sum to 10000"));
    }

    #[test]
    fn schema_v4_fast_relay_is_bounded_fail_closed_and_dependency_checked() {
        let defaults = parse_config("version: 4\n").unwrap();
        assert!(!defaults.fast_mode.relay.fast_compat_enabled);
        assert_eq!(defaults.fast_mode.relay.authorization_ttl_seconds, 90);
        assert_eq!(defaults.fast_mode.relay.max_bitrate_kbps, 50_000);

        let old =
            parse_config("version: 3\nfast_mode:\n  relay:\n    fast_compat_enabled: false\n")
                .unwrap_err();
        assert!(old.contains("upgrade the document to version 4"));

        let bad_ttl =
            parse_config("version: 4\nfast_mode:\n  relay:\n    authorization_ttl_seconds: 29\n")
                .unwrap_err();
        assert!(bad_ttl.contains("between 30 and 300"));

        let bad_bitrate =
            parse_config("version: 4\nfast_mode:\n  relay:\n    max_bitrate_kbps: 200001\n")
                .unwrap_err();
        assert!(bad_bitrate.contains("between 1000 and 200000"));

        let missing_quality =
            parse_config("version: 4\nfast_mode:\n  relay:\n    fast_compat_enabled: true\n")
                .unwrap_err();
        assert!(missing_quality.contains("requires relay_quality.enabled"));

        let insecure = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks: { file: auth/jwks.json }
relay_quality: { enabled: true }
fast_mode:
  relay: { fast_compat_enabled: true }
"#;
        let insecure = parse_config(insecure).unwrap_err();
        assert!(insecure.contains("requires Secure TCP or WebSocket signalling"));

        let enabled = r#"
version: 4
relay_servers: [relay-a:21117, relay-b:21117]
secure_tcp: { mode: auto }
websocket_signal:
  relay_health:
    endpoints:
      - { relay: relay-a:21117, url: wss://relay-a.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
      - { relay: relay-b:21117, url: wss://relay-b.example/ws/telemetry, telemetry_secret_file: /run/secrets/relay-telemetry }
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks: { file: auth/jwks.json }
relay_quality:
  enabled: true
fast_mode:
  relay:
    fast_compat_enabled: true
    authorization_ttl_seconds: 120
    max_bitrate_kbps: 80000
"#;
        let config = parse_config(enabled).unwrap();
        assert!(config.fast_mode.relay.fast_compat_enabled);
        assert_eq!(config.fast_mode.relay.authorization_ttl_seconds, 120);
        assert_eq!(config.fast_mode.relay.max_bitrate_kbps, 80_000);
    }

    #[test]
    fn enforce_requires_an_initial_jwks_file() {
        let raw = r#"
version: 3
connection_auth:
  mode: enforce
  issuer: https://api.example.com
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  jwks:
    url: https://api.example.com/api/internal/v1/auth/jwks
    ca_file: auth/ca.pem
    cert_file: auth/client.pem
    key_file: auth/client-key.pem
    server_name: api.example.com
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("jwks.file is required in enforce mode"));
    }

    #[test]
    fn every_configured_jwks_endpoint_requires_exact_mtls_references() {
        let missing_identity = r#"
version: 3
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks:
    file: auth/jwks.json
    url: https://api.example.com/api/internal/v1/auth/jwks
"#;
        let err = parse_config(missing_identity).unwrap_err();
        assert!(err.contains("jwks.ca_file is required"));

        let wrong_name = r#"
version: 3
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks:
    file: auth/jwks.json
    url: https://api.example.com/api/internal/v1/auth/jwks
    ca_file: auth/ca.pem
    cert_file: auth/client.pem
    key_file: auth/client-key.pem
    server_name: other.example.com
"#;
        let err = parse_config(wrong_name).unwrap_err();
        assert!(err.contains("jwks.url host must equal configured server_name"));
    }

    #[test]
    fn required_introspection_requires_complete_mtls_references() {
        let raw = r#"
version: 3
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks:
    file: auth/jwks.json
  introspection:
    required: true
    url: https://api.example.com/api/internal/v1/auth/introspect
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("introspection.ca_file is required"));
    }

    #[test]
    fn every_configured_introspection_endpoint_requires_mtls_references() {
        let raw = r#"
version: 3
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks:
    file: auth/jwks.json
  introspection:
    required: false
    url: https://api.example.com/api/internal/v1/auth/introspect
"#;
        let err = parse_config(raw).unwrap_err();
        assert!(err.contains("introspection.ca_file is required"));
    }

    #[test]
    fn parsed_and_effective_digests_have_distinct_semantics() {
        let first = validate_config(parse_document(b"version: 3\n").unwrap()).unwrap();
        let second =
            validate_config(parse_document(b"# formatting change\nversion: 3\n").unwrap()).unwrap();
        assert_ne!(first.source_digest, second.source_digest);
        assert_eq!(first.effective_digest, second.effective_digest);
    }

    #[test]
    fn must_login_is_an_enforce_floor() {
        assert_eq!(
            effective_connection_auth_mode(ConnectionAuthMode::Off, true),
            ConnectionAuthMode::Enforce
        );
        assert_eq!(
            effective_connection_auth_mode(ConnectionAuthMode::Audit, false),
            ConnectionAuthMode::Audit
        );
    }

    #[test]
    fn subsystem_acknowledgement_does_not_advance_generation() {
        let _guard = STATE_TEST_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "starry-config-subsystem-ack-{}",
            std::process::id()
        ));
        let path = directory.join("config.yaml");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "version: 3\nrelay_servers: [relay-a]\n").unwrap();
        let startup = initialize(path.to_str().unwrap());
        let generation = startup.activation_ack.unwrap().generation;
        let ack = acknowledge_active(
            generation,
            vec![SubsystemAck {
                subsystem: "relay_pool".to_owned(),
                accepted: true,
                detail: "applied".to_owned(),
            }],
        )
        .unwrap();
        assert_eq!(ack.generation, generation);
        assert_eq!(runtime_state().generation, generation);
        assert_eq!(runtime_state().subsystem_acks, ack.subsystem_acks);
        let _ = fs::remove_dir_all(directory);
    }
}
