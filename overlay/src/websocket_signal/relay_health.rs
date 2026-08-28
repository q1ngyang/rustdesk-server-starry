use super::RelayRequirement;
use crate::starry_config::RelayHealthConfig;
use hbb_common::{
    futures_util::SinkExt,
    log, timeout,
    tokio::{self, sync::Notify, time::Duration},
};
use once_cell::sync::Lazy;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        RwLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static CLOCK: Lazy<Instant> = Lazy::new(Instant::now);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);
static STATE: Lazy<RwLock<HealthState>> = Lazy::new(|| RwLock::new(HealthState::default()));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Unknown,
    Healthy,
    Unhealthy,
}

#[derive(Clone, Debug)]
struct EndpointState {
    status: Status,
    consecutive_successes: u32,
    consecutive_failures: u32,
    last_success_millis: Option<u64>,
    last_failure_millis: Option<u64>,
    last_probe_unix_millis: Option<u64>,
    latency_ms: Option<u64>,
    version: Option<String>,
    last_error_code: Option<&'static str>,
    last_error: Option<String>,
}

impl Default for EndpointState {
    fn default() -> Self {
        Self {
            status: Status::Unknown,
            consecutive_successes: 0,
            consecutive_failures: 0,
            last_success_millis: None,
            last_failure_millis: None,
            last_probe_unix_millis: None,
            latency_ms: None,
            version: None,
            last_error_code: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug)]
struct ProbeFailure {
    code: &'static str,
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProbeSuccess {
    latency_ms: u64,
    version: Option<String>,
}

impl ProbeFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: sanitize_error(&message.into()),
        }
    }
}

#[derive(Default)]
struct HealthState {
    enabled: bool,
    generation: u64,
    completed_generation: u64,
    snapshot_id: u64,
    config: RelayHealthConfig,
    endpoints: HashMap<String, EndpointState>,
}

#[derive(Clone, Debug)]
pub(crate) struct HealthSnapshot {
    pub(crate) relay: String,
    pub(crate) status: &'static str,
    pub(crate) last_success_age_seconds: Option<u64>,
    pub(crate) last_failure_age_seconds: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeEndpointSnapshot {
    pub(crate) relay: String,
    pub(crate) url: String,
    pub(crate) state: &'static str,
    pub(crate) last_probe_at: Option<String>,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) version: Option<String>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) consecutive_successes: u32,
    pub(crate) consecutive_failures: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeHealthSnapshot {
    pub(crate) enabled: bool,
    pub(crate) generation: u64,
    pub(crate) completed_generation: u64,
    pub(crate) snapshot_id: u64,
    pub(crate) endpoints: HashMap<String, RuntimeEndpointSnapshot>,
}

impl RuntimeHealthSnapshot {
    pub(crate) fn is_ready(&self) -> bool {
        self.enabled && self.completed_generation == self.generation
    }

    pub(crate) fn endpoint(&self, relay: &str) -> Option<&RuntimeEndpointSnapshot> {
        self.endpoints.get(&relay.to_ascii_lowercase())
    }

    pub(crate) fn is_healthy(&self, relay: &str) -> bool {
        self.is_ready()
            && self
                .endpoint(relay)
                .map(|endpoint| endpoint.state == "healthy")
                .unwrap_or(false)
    }
}

pub(crate) fn reconfigure(enabled: bool, config: &RelayHealthConfig) -> Result<String, String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let endpoints = config
        .endpoints
        .iter()
        .map(|endpoint| {
            (
                endpoint.relay.to_ascii_lowercase(),
                EndpointState::default(),
            )
        })
        .collect();
    match STATE.write() {
        Ok(mut state) => {
            *state = HealthState {
                enabled,
                generation,
                completed_generation: 0,
                snapshot_id: next_snapshot_id(),
                config: config.clone(),
                endpoints,
            };
        }
        Err(err) => return Err(format!("WebSocket Relay health lock failed: {err}")),
    }
    start_task();
    WAKE.notify_one();
    if enabled {
        Ok(format!(
            "WebSocket Relay health generation {generation} scheduled for immediate probe"
        ))
    } else {
        Ok("WebSocket Relay health disabled".to_owned())
    }
}

fn start_task() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async {
        loop {
            let snapshot = STATE
                .read()
                .ok()
                .map(|state| (state.enabled, state.generation, state.config.clone()));
            let Some((enabled, generation, config)) = snapshot else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            if enabled {
                run_cycle(generation, config.clone()).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(config.interval_seconds)) => {}
                    _ = WAKE.notified() => {}
                }
            } else {
                WAKE.notified().await;
            }
        }
    });
}

async fn run_cycle(generation: u64, config: RelayHealthConfig) {
    for endpoint in &config.endpoints {
        let result = probe(&endpoint.url, config.timeout_ms).await;
        record(
            generation,
            &endpoint.relay,
            result,
            config.success_threshold,
            config.failure_threshold,
        );
    }
    if let Ok(mut state) = STATE.write() {
        if state.generation == generation && state.completed_generation != generation {
            state.completed_generation = generation;
            state.snapshot_id = next_snapshot_id();
        }
    }
}

async fn probe(url: &str, timeout_ms: u64) -> Result<ProbeSuccess, ProbeFailure> {
    let started = Instant::now();
    let connected = timeout(timeout_ms, tokio_tungstenite::connect_async(url)).await;
    let (mut stream, response) = connected
        .map_err(|_| ProbeFailure::new("probe_timeout", "probe timeout"))?
        .map_err(|err| ProbeFailure::new("connect_failed", err.to_string()))?;
    if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
        return Err(ProbeFailure::new(
            "unexpected_http_status",
            format!("unexpected HTTP status {}", response.status()),
        ));
    }
    stream
        .send(tungstenite::Message::Close(None))
        .await
        .map_err(|err| ProbeFailure::new("close_failed", err.to_string()))?;
    Ok(ProbeSuccess {
        latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        version: relay_version(&response),
    })
}

fn relay_version<T>(response: &http::Response<T>) -> Option<String> {
    let version = response.headers().get("x-starry-version")?.to_str().ok()?;
    if version.is_empty()
        || version.len() > 128
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        return None;
    }
    Some(version.to_owned())
}

fn record(
    generation: u64,
    relay: &str,
    result: Result<ProbeSuccess, ProbeFailure>,
    success_threshold: u32,
    failure_threshold: u32,
) {
    let Ok(mut state) = STATE.write() else {
        return;
    };
    if state.generation != generation {
        return;
    }
    let Some(endpoint) = state.endpoints.get_mut(&relay.to_ascii_lowercase()) else {
        return;
    };
    endpoint.last_probe_unix_millis = Some(unix_millis());
    match result {
        Ok(success) => {
            endpoint.consecutive_successes = endpoint.consecutive_successes.saturating_add(1);
            endpoint.consecutive_failures = 0;
            endpoint.last_success_millis = Some(now_millis());
            endpoint.latency_ms = Some(success.latency_ms);
            endpoint.version = success.version;
            endpoint.last_error_code = None;
            endpoint.last_error = None;
            if endpoint.consecutive_successes >= success_threshold {
                endpoint.status = Status::Healthy;
            }
        }
        Err(err) => {
            endpoint.consecutive_failures = endpoint.consecutive_failures.saturating_add(1);
            endpoint.consecutive_successes = 0;
            endpoint.last_failure_millis = Some(now_millis());
            endpoint.latency_ms = None;
            endpoint.last_error_code = Some(err.code);
            endpoint.last_error = Some(err.message.clone());
            if endpoint.consecutive_failures >= failure_threshold {
                endpoint.status = Status::Unhealthy;
            }
            log::warn!(
                "WebSocket Relay probe failed for {relay} ({}): {}",
                err.code,
                err.message
            );
        }
    }
    state.snapshot_id = next_snapshot_id();
}

pub(crate) fn ready() -> bool {
    STATE
        .read()
        .map(|state| {
            state.enabled
                && state.completed_generation == state.generation
                && state
                    .endpoints
                    .values()
                    .any(|endpoint| endpoint.status == Status::Healthy)
        })
        .unwrap_or(false)
}

pub(crate) fn snapshot_id() -> u64 {
    STATE
        .read()
        .map(|state| state.snapshot_id)
        .unwrap_or_default()
}

pub(crate) fn runtime_snapshot() -> RuntimeHealthSnapshot {
    let Ok(state) = STATE.read() else {
        return RuntimeHealthSnapshot {
            enabled: false,
            generation: 0,
            completed_generation: 0,
            snapshot_id: 0,
            endpoints: HashMap::new(),
        };
    };
    let endpoints = state
        .config
        .endpoints
        .iter()
        .map(|configured| {
            let endpoint = state
                .endpoints
                .get(&configured.relay.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            (
                configured.relay.to_ascii_lowercase(),
                RuntimeEndpointSnapshot {
                    relay: configured.relay.clone(),
                    url: configured.url.clone(),
                    state: status_name(endpoint.status),
                    last_probe_at: endpoint.last_probe_unix_millis.and_then(rfc3339_millis),
                    latency_ms: endpoint.latency_ms,
                    version: endpoint.version,
                    error_code: endpoint.last_error_code.map(str::to_owned),
                    error_message: endpoint.last_error,
                    consecutive_successes: endpoint.consecutive_successes,
                    consecutive_failures: endpoint.consecutive_failures,
                },
            )
        })
        .collect();
    RuntimeHealthSnapshot {
        enabled: state.enabled,
        generation: state.generation,
        completed_generation: state.completed_generation,
        snapshot_id: state.snapshot_id,
        endpoints,
    }
}

pub(crate) fn eligible_relays(
    configured_relays: &[String],
    native_online: &[String],
    requirement: RelayRequirement,
) -> Vec<String> {
    if requirement == RelayRequirement::NativeOnly {
        return native_online.to_vec();
    }
    let native: HashSet<String> = native_online
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let state = match STATE.read() {
        Ok(state) if state.enabled && state.completed_generation == state.generation => state,
        _ => return Vec::new(),
    };
    configured_relays
        .iter()
        .filter(|relay| {
            let key = relay.to_ascii_lowercase();
            let websocket_healthy = state
                .endpoints
                .get(&key)
                .map(|endpoint| endpoint.status == Status::Healthy)
                .unwrap_or(false);
            websocket_healthy
                && (requirement == RelayRequirement::WebSocketOnly || native.contains(&key))
        })
        .cloned()
        .collect()
}

pub(crate) fn snapshots() -> Vec<HealthSnapshot> {
    let Ok(state) = STATE.read() else {
        return Vec::new();
    };
    let now = now_millis();
    state
        .config
        .endpoints
        .iter()
        .map(|configured| {
            let endpoint = state
                .endpoints
                .get(&configured.relay.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            HealthSnapshot {
                relay: configured.relay.clone(),
                status: status_name(endpoint.status),
                last_success_age_seconds: endpoint
                    .last_success_millis
                    .map(|instant| now.saturating_sub(instant) / 1_000),
                last_failure_age_seconds: endpoint
                    .last_failure_millis
                    .map(|instant| now.saturating_sub(instant) / 1_000),
                last_error: endpoint.last_error,
            }
        })
        .collect()
}

fn now_millis() -> u64 {
    CLOCK.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn next_snapshot_id() -> u64 {
    SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1
}

fn rfc3339_millis(value: u64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value as i64)
        .map(|timestamp| timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Unknown => "unknown",
        Status::Healthy => "healthy",
        Status::Unhealthy => "unhealthy",
    }
}

fn sanitize_error(error: &str) -> String {
    let mut value = error.replace('\r', " ").replace('\n', " ");
    value.truncate(240);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starry_config::RelayEndpointConfig;

    #[test]
    fn native_filter_is_unchanged() {
        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let online = vec!["relay-b".to_owned()];
        assert_eq!(
            eligible_relays(&configured, &online, RelayRequirement::NativeOnly),
            online
        );
    }

    #[test]
    fn thresholds_generation_and_mixed_health_are_enforced() {
        let relay = "relay-a.example.com:21117".to_owned();
        let config = RelayHealthConfig {
            success_threshold: 2,
            failure_threshold: 2,
            endpoints: vec![RelayEndpointConfig {
                relay: relay.clone(),
                url: "wss://relay-a.example.com/ws/relay".to_owned(),
            }],
            ..Default::default()
        };
        *STATE.write().unwrap() = HealthState {
            enabled: true,
            generation: 42,
            completed_generation: 42,
            snapshot_id: 7,
            config,
            endpoints: HashMap::from([(relay.to_ascii_lowercase(), EndpointState::default())]),
        };
        let configured = vec![relay.clone()];
        let native_online = vec![relay.clone()];

        record(
            41,
            &relay,
            probe_success(10, Some("1.1.16-patch-v1.2.1")),
            2,
            2,
        );
        assert_eq!(snapshot_id(), 7);
        assert!(
            eligible_relays(&configured, &native_online, RelayRequirement::WebSocketOnly)
                .is_empty()
        );
        record(
            42,
            &relay,
            probe_success(10, Some("1.1.16-patch-v1.2.1")),
            2,
            2,
        );
        let first_probe_snapshot = snapshot_id();
        assert_ne!(first_probe_snapshot, 7);
        assert!(
            eligible_relays(&configured, &native_online, RelayRequirement::WebSocketOnly)
                .is_empty()
        );
        record(
            42,
            &relay,
            probe_success(10, Some("1.1.16-patch-v1.2.1")),
            2,
            2,
        );
        assert_ne!(snapshot_id(), first_probe_snapshot);
        let runtime = runtime_snapshot();
        assert_eq!(
            runtime
                .endpoints
                .get(&relay.to_ascii_lowercase())
                .and_then(|endpoint| endpoint.version.as_deref()),
            Some("1.1.16-patch-v1.2.1")
        );
        assert_eq!(
            eligible_relays(&configured, &native_online, RelayRequirement::WebSocketOnly),
            configured
        );
        assert!(eligible_relays(&configured, &[], RelayRequirement::Mixed).is_empty());
        assert_eq!(
            eligible_relays(&configured, &native_online, RelayRequirement::Mixed),
            native_online
        );

        record(
            42,
            &relay,
            Err(ProbeFailure::new("test_failure", "first")),
            2,
            2,
        );
        assert!(
            !eligible_relays(&configured, &native_online, RelayRequirement::WebSocketOnly)
                .is_empty()
        );
        record(
            42,
            &relay,
            Err(ProbeFailure::new("test_failure", "second")),
            2,
            2,
        );
        assert!(
            eligible_relays(&configured, &native_online, RelayRequirement::WebSocketOnly)
                .is_empty()
        );
    }

    fn probe_success(latency_ms: u64, version: Option<&str>) -> Result<ProbeSuccess, ProbeFailure> {
        Ok(ProbeSuccess {
            latency_ms,
            version: version.map(str::to_owned),
        })
    }

    #[test]
    fn relay_version_header_is_bounded_and_validated() {
        let response = http::Response::builder()
            .header("x-starry-version", "1.1.16-patch-v1.2.1")
            .body(())
            .unwrap();
        assert_eq!(
            relay_version(&response).as_deref(),
            Some("1.1.16-patch-v1.2.1")
        );

        let response = http::Response::builder()
            .header("x-starry-version", "invalid version")
            .body(())
            .unwrap();
        assert_eq!(relay_version(&response), None);
    }
}
