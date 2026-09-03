use super::RelayRequirement;
use crate::starry_config::{RelayEndpointConfig, RelayHealthConfig};
use hbb_common::{
    futures_util::{stream, SinkExt, StreamExt},
    log, timeout,
    tokio::{self, sync::Notify, time::Duration},
};
use once_cell::sync::Lazy;
use serde_derive::Deserialize;
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::auth;
use std::{
    collections::{HashMap, HashSet},
    fs,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        RwLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tungstenite::client::IntoClientRequest;

static CLOCK: Lazy<Instant> = Lazy::new(Instant::now);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);
static STATE: Lazy<RwLock<HealthState>> = Lazy::new(|| RwLock::new(HealthState::default()));
const MAX_CONCURRENT_HEALTH_PROBES: usize = 8;
const TELEMETRY_SCHEMA_V1: u32 = 1;
const TELEMETRY_SCHEMA_V2: u32 = 2;
const TELEMETRY_CLOCK_SKEW_MILLIS: u64 = 30_000;
const MIN_TELEMETRY_SECRET_BYTES: usize = 32;
const MAX_TELEMETRY_SECRET_BYTES: usize = 1_024;

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
    relay_probe_protocol: Option<u32>,
    relay_load_protocol: Option<u32>,
    telemetry_observed_unix_millis: Option<u64>,
    telemetry_instance_id: Option<String>,
    telemetry_sequence: Option<u64>,
    telemetry_uptime_seconds: Option<u64>,
    load: Option<RelayLoadTelemetry>,
    telemetry_restarts: u64,
    last_restart_unix_millis: Option<u64>,
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
            relay_probe_protocol: None,
            relay_load_protocol: None,
            telemetry_observed_unix_millis: None,
            telemetry_instance_id: None,
            telemetry_sequence: None,
            telemetry_uptime_seconds: None,
            load: None,
            telemetry_restarts: 0,
            last_restart_unix_millis: None,
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
    relay_probe_protocol: Option<u32>,
    relay_load_protocol: Option<u32>,
    load: Option<RelayLoadTelemetry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RelayLoadTelemetry {
    pub(crate) telemetry_schema: u32,
    pub(crate) process_instance_id: String,
    pub(crate) sequence: u64,
    pub(crate) observed_at_unix_ms: u64,
    pub(crate) uptime_seconds: u64,
    pub(crate) load_basis_points: u32,
    pub(crate) active_sessions: u32,
    pub(crate) pending_pairs: u32,
    pub(crate) capacity_sessions: u32,
    pub(crate) bandwidth_bps: u64,
    pub(crate) bandwidth_ema_alpha_basis_points: u32,
    pub(crate) capacity_bandwidth_bps: u64,
    pub(crate) draining: bool,
    pub(crate) admission_open: bool,
    pub(crate) admission_rejections: u64,
    pub(crate) probe_malformed: u64,
    pub(crate) probe_unsupported: u64,
    pub(crate) probe_rate_limited: u64,
    pub(crate) probe_successful: u64,
    pub(crate) telemetry_auth_failures: u64,
    pub(crate) fast_media_relay_udp: Option<u32>,
    pub(crate) fast_media_udp_enabled: Option<bool>,
    pub(crate) fast_media_udp_healthy: Option<bool>,
    pub(crate) fast_media_udp_port: Option<u16>,
    pub(crate) fast_media_active_allocations: Option<u64>,
    pub(crate) fast_media_active_streams: Option<u64>,
    pub(crate) fast_media_hello_accepted: Option<u64>,
    pub(crate) fast_media_cookie_rejected: Option<u64>,
    pub(crate) fast_media_bind_succeeded: Option<u64>,
    pub(crate) fast_media_bind_rejected: Option<u64>,
    pub(crate) fast_media_grant_rejected: Option<u64>,
    pub(crate) fast_media_role_mismatch: Option<u64>,
    pub(crate) fast_media_session_mismatch: Option<u64>,
    pub(crate) fast_media_allocation_mismatch: Option<u64>,
    pub(crate) fast_media_rebinds: Option<u64>,
    pub(crate) fast_media_forwarded_packets: Option<u64>,
    pub(crate) fast_media_forwarded_bytes: Option<u64>,
    pub(crate) fast_media_dropped_packets: Option<u64>,
    pub(crate) fast_media_rate_limited: Option<u64>,
    pub(crate) fast_media_replay_rejected: Option<u64>,
    pub(crate) fast_media_expired_allocations: Option<u64>,
    pub(crate) fast_media_listener_failures: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TelemetryEnvelope {
    telemetry_schema: u32,
    process_instance_id: String,
    sequence: u64,
    observed_at_unix_ms: u64,
    uptime_seconds: u64,
    version: String,
    relay_probe_protocol: u32,
    relay_load_protocol: u32,
    load_basis_points: u32,
    active_sessions: u32,
    pending_pairs: u32,
    capacity_sessions: u32,
    bandwidth_bps: u64,
    bandwidth_ema_alpha_basis_points: u32,
    capacity_bandwidth_bps: u64,
    draining: bool,
    admission_open: bool,
    admission_rejections: u64,
    probe_malformed: u64,
    probe_unsupported: u64,
    probe_rate_limited: u64,
    probe_successful: u64,
    telemetry_auth_failures: u64,
    #[serde(default)]
    fast_media: Option<FastMediaTelemetryEnvelope>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FastMediaTelemetryEnvelope {
    protocol: u32,
    enabled: bool,
    healthy: bool,
    udp_port: u16,
    active_allocations: u64,
    active_streams: u64,
    hello_accepted: u64,
    cookie_rejected: u64,
    bind_succeeded: u64,
    bind_rejected: u64,
    grant_rejected: u64,
    role_mismatch: u64,
    session_mismatch: u64,
    allocation_mismatch: u64,
    rebinds: u64,
    forwarded_packets: u64,
    forwarded_bytes: u64,
    dropped_packets: u64,
    rate_limited: u64,
    replay_rejected: u64,
    expired_allocations: u64,
    listener_failures: u64,
}

struct TelemetryRequestContext {
    nonce: String,
    key: auth::Key,
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
    pub(crate) relay_probe_protocol: Option<u32>,
    pub(crate) relay_load_protocol: Option<u32>,
    pub(crate) observed_at: Option<String>,
    pub(crate) age_seconds: Option<u64>,
    pub(crate) stale: bool,
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
    pub(crate) observed_at: Option<String>,
    pub(crate) observed_at_unix_ms: Option<u64>,
    pub(crate) age_seconds: Option<u64>,
    pub(crate) stale: bool,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) version: Option<String>,
    pub(crate) relay_probe_protocol: Option<u32>,
    pub(crate) relay_load_protocol: Option<u32>,
    pub(crate) telemetry_instance_id: Option<String>,
    pub(crate) telemetry_sequence: Option<u64>,
    pub(crate) telemetry_uptime_seconds: Option<u64>,
    pub(crate) load: Option<RelayLoadTelemetry>,
    pub(crate) telemetry_restarts: u64,
    pub(crate) last_restart_at: Option<String>,
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
    let success_threshold = config.success_threshold;
    let failure_threshold = config.failure_threshold;
    let timeout_ms = config.timeout_ms;
    let probes = collect_bounded(
        config.endpoints,
        MAX_CONCURRENT_HEALTH_PROBES,
        |endpoint| async move {
            let result = probe(&endpoint, timeout_ms).await;
            (endpoint.relay, result)
        },
    )
    .await;
    for (relay, result) in probes {
        record(
            generation,
            &relay,
            result,
            success_threshold,
            failure_threshold,
        );
    }
    if let Ok(mut state) = STATE.write() {
        if state.generation == generation && state.completed_generation != generation {
            state.completed_generation = generation;
            state.snapshot_id = next_snapshot_id();
        }
    }
}

async fn collect_bounded<T, R, I, F, Fut>(items: I, maximum: usize, operation: F) -> Vec<R>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = R>,
{
    stream::iter(items)
        .map(operation)
        .buffer_unordered(maximum.max(1))
        .collect()
        .await
}

async fn probe(
    endpoint: &RelayEndpointConfig,
    timeout_ms: u64,
) -> Result<ProbeSuccess, ProbeFailure> {
    let started = Instant::now();
    let (request, telemetry_context) = telemetry_request(endpoint)?;
    let connected = timeout(timeout_ms, tokio_tungstenite::connect_async(request)).await;
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
    let public_probe_protocol = capability_version(&response, "x-starry-relay-probe-protocol");
    let public_load_protocol = capability_version(&response, "x-starry-relay-load-protocol");
    let (version, relay_probe_protocol, relay_load_protocol, load) = if let Some(context) =
        telemetry_context
    {
        let (version, probe_protocol, load_protocol, load) = relay_telemetry(&response, &context)?;
        if public_probe_protocol != Some(probe_protocol)
            || public_load_protocol != Some(load_protocol)
        {
            return Err(ProbeFailure::new(
                "telemetry_capability_mismatch",
                "signed telemetry and public capability headers differ",
            ));
        }
        (
            Some(version),
            Some(probe_protocol),
            Some(load_protocol),
            Some(load),
        )
    } else {
        (
            relay_version(&response),
            public_probe_protocol,
            public_load_protocol,
            None,
        )
    };
    Ok(ProbeSuccess {
        latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        version,
        relay_probe_protocol,
        relay_load_protocol,
        load,
    })
}

fn telemetry_request(
    endpoint: &RelayEndpointConfig,
) -> Result<(http::Request<()>, Option<TelemetryRequestContext>), ProbeFailure> {
    let mut request = endpoint
        .url
        .clone()
        .into_client_request()
        .map_err(|err| ProbeFailure::new("invalid_probe_url", err.to_string()))?;
    let Some(secret_file) = endpoint.telemetry_secret_file.as_deref() else {
        return Ok((request, None));
    };
    let key = telemetry_key(secret_file)?;
    let timestamp = unix_millis() / 1_000;
    let nonce = uuid::Uuid::now_v7().simple().to_string();
    let canonical = format!("starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry");
    let signature = hex_encode(auth::authenticate(canonical.as_bytes(), &key).as_ref());
    for (name, value) in [
        ("x-starry-telemetry-timestamp", timestamp.to_string()),
        ("x-starry-telemetry-nonce", nonce.clone()),
        ("x-starry-telemetry-auth", signature),
    ] {
        request.headers_mut().insert(
            http::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| ProbeFailure::new("telemetry_request_invalid", "invalid header"))?,
            http::HeaderValue::from_str(&value).map_err(|_| {
                ProbeFailure::new("telemetry_request_invalid", "invalid header value")
            })?,
        );
    }
    Ok((request, Some(TelemetryRequestContext { nonce, key })))
}

fn telemetry_key(secret_file: &str) -> Result<auth::Key, ProbeFailure> {
    sodiumoxide::init().map_err(|_| {
        ProbeFailure::new(
            "telemetry_crypto_unavailable",
            "telemetry authentication initialization failed",
        )
    })?;
    let mut secret = fs::read(secret_file).map_err(|err| {
        ProbeFailure::new(
            "telemetry_secret_unavailable",
            format!("cannot read telemetry secret file: {err}"),
        )
    })?;
    while matches!(secret.last(), Some(b'\n' | b'\r')) {
        secret.pop();
    }
    if !(MIN_TELEMETRY_SECRET_BYTES..=MAX_TELEMETRY_SECRET_BYTES).contains(&secret.len()) {
        return Err(ProbeFailure::new(
            "telemetry_secret_invalid",
            "telemetry secret must contain 32..1024 bytes",
        ));
    }
    let digest = Sha256::digest(&secret);
    secret.fill(0);
    auth::Key::from_slice(&digest).ok_or_else(|| {
        ProbeFailure::new(
            "telemetry_secret_invalid",
            "telemetry key derivation failed",
        )
    })
}

fn relay_telemetry<T>(
    response: &http::Response<T>,
    context: &TelemetryRequestContext,
) -> Result<(String, u32, u32, RelayLoadTelemetry), ProbeFailure> {
    let payload = bounded_header(response, "x-starry-telemetry", 8_192).ok_or_else(|| {
        ProbeFailure::new(
            "telemetry_missing",
            "authenticated telemetry payload is missing",
        )
    })?;
    let signature = bounded_header(response, "x-starry-telemetry-auth", 128).ok_or_else(|| {
        ProbeFailure::new(
            "telemetry_auth_missing",
            "telemetry response signature is missing",
        )
    })?;
    let tag = hex_decode_tag(signature).ok_or_else(|| {
        ProbeFailure::new(
            "telemetry_auth_invalid",
            "telemetry response signature is invalid",
        )
    })?;
    let canonical = format!(
        "starry-telemetry-response-v1\n{}\n{}",
        context.nonce, payload
    );
    if !auth::verify(&tag, canonical.as_bytes(), &context.key) {
        return Err(ProbeFailure::new(
            "telemetry_auth_invalid",
            "telemetry response signature verification failed",
        ));
    }
    let decoded = base64::decode_config(payload, base64::URL_SAFE_NO_PAD).map_err(|_| {
        ProbeFailure::new(
            "telemetry_payload_invalid",
            "telemetry payload is not base64url",
        )
    })?;
    if decoded.len() > 6_144 {
        return Err(ProbeFailure::new(
            "telemetry_payload_invalid",
            "telemetry payload exceeds the decoded size limit",
        ));
    }
    let envelope: TelemetryEnvelope = serde_json::from_slice(&decoded).map_err(|_| {
        ProbeFailure::new(
            "telemetry_payload_invalid",
            "telemetry payload is not valid schema v1",
        )
    })?;
    validate_telemetry(&envelope)?;
    let version = validate_version(&envelope.version).ok_or_else(|| {
        ProbeFailure::new("telemetry_payload_invalid", "telemetry version is invalid")
    })?;
    let probe_protocol = envelope.relay_probe_protocol;
    let load_protocol = envelope.relay_load_protocol;
    let fast_media = envelope.fast_media;
    Ok((
        version,
        probe_protocol,
        load_protocol,
        RelayLoadTelemetry {
            telemetry_schema: envelope.telemetry_schema,
            process_instance_id: envelope.process_instance_id,
            sequence: envelope.sequence,
            observed_at_unix_ms: envelope.observed_at_unix_ms,
            uptime_seconds: envelope.uptime_seconds,
            load_basis_points: envelope.load_basis_points,
            active_sessions: envelope.active_sessions,
            pending_pairs: envelope.pending_pairs,
            capacity_sessions: envelope.capacity_sessions,
            bandwidth_bps: envelope.bandwidth_bps,
            bandwidth_ema_alpha_basis_points: envelope.bandwidth_ema_alpha_basis_points,
            capacity_bandwidth_bps: envelope.capacity_bandwidth_bps,
            draining: envelope.draining,
            admission_open: envelope.admission_open,
            admission_rejections: envelope.admission_rejections,
            probe_malformed: envelope.probe_malformed,
            probe_unsupported: envelope.probe_unsupported,
            probe_rate_limited: envelope.probe_rate_limited,
            probe_successful: envelope.probe_successful,
            telemetry_auth_failures: envelope.telemetry_auth_failures,
            fast_media_relay_udp: fast_media.as_ref().map(|value| value.protocol),
            fast_media_udp_enabled: fast_media.as_ref().map(|value| value.enabled),
            fast_media_udp_healthy: fast_media.as_ref().map(|value| value.healthy),
            fast_media_udp_port: fast_media.as_ref().map(|value| value.udp_port),
            fast_media_active_allocations: fast_media
                .as_ref()
                .map(|value| value.active_allocations),
            fast_media_active_streams: fast_media.as_ref().map(|value| value.active_streams),
            fast_media_hello_accepted: fast_media.as_ref().map(|value| value.hello_accepted),
            fast_media_cookie_rejected: fast_media.as_ref().map(|value| value.cookie_rejected),
            fast_media_bind_succeeded: fast_media.as_ref().map(|value| value.bind_succeeded),
            fast_media_bind_rejected: fast_media.as_ref().map(|value| value.bind_rejected),
            fast_media_grant_rejected: fast_media.as_ref().map(|value| value.grant_rejected),
            fast_media_role_mismatch: fast_media.as_ref().map(|value| value.role_mismatch),
            fast_media_session_mismatch: fast_media.as_ref().map(|value| value.session_mismatch),
            fast_media_allocation_mismatch: fast_media
                .as_ref()
                .map(|value| value.allocation_mismatch),
            fast_media_rebinds: fast_media.as_ref().map(|value| value.rebinds),
            fast_media_forwarded_packets: fast_media.as_ref().map(|value| value.forwarded_packets),
            fast_media_forwarded_bytes: fast_media.as_ref().map(|value| value.forwarded_bytes),
            fast_media_dropped_packets: fast_media.as_ref().map(|value| value.dropped_packets),
            fast_media_rate_limited: fast_media.as_ref().map(|value| value.rate_limited),
            fast_media_replay_rejected: fast_media.as_ref().map(|value| value.replay_rejected),
            fast_media_expired_allocations: fast_media
                .as_ref()
                .map(|value| value.expired_allocations),
            fast_media_listener_failures: fast_media.as_ref().map(|value| value.listener_failures),
        },
    ))
}

fn validate_telemetry(envelope: &TelemetryEnvelope) -> Result<(), ProbeFailure> {
    let now = unix_millis();
    let instance_valid = !envelope.process_instance_id.is_empty()
        && envelope.process_instance_id.len() <= 64
        && envelope
            .process_instance_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    let schema_valid = match envelope.telemetry_schema {
        TELEMETRY_SCHEMA_V1 => envelope.fast_media.is_none(),
        TELEMETRY_SCHEMA_V2 => envelope.fast_media.as_ref().is_some_and(|fast_media| {
            fast_media.protocol == 1
                && (!fast_media.healthy || (fast_media.enabled && fast_media.udp_port > 0))
                && (!fast_media.enabled || fast_media.udp_port > 0)
        }),
        _ => false,
    };
    let structurally_valid = schema_valid
        && instance_valid
        && envelope.sequence > 0
        && envelope.observed_at_unix_ms <= now.saturating_add(TELEMETRY_CLOCK_SKEW_MILLIS)
        && envelope.relay_probe_protocol > 0
        && envelope.relay_load_protocol > 0
        && envelope.load_basis_points <= 10_000
        && envelope.active_sessions <= envelope.capacity_sessions
        && envelope.capacity_sessions > 0
        && envelope.capacity_bandwidth_bps > 0
        && (1..=10_000).contains(&envelope.bandwidth_ema_alpha_basis_points)
        && envelope.admission_open
            == (!envelope.draining && envelope.active_sessions < envelope.capacity_sessions);
    if structurally_valid {
        Ok(())
    } else {
        Err(ProbeFailure::new(
            "telemetry_payload_invalid",
            "telemetry fields violate supported schema bounds or invariants",
        ))
    }
}

fn capability_version<T>(response: &http::Response<T>, name: &str) -> Option<u32> {
    header_u64(response, name, u32::MAX as u64)
        .and_then(|value| (value > 0).then_some(value as u32))
}

fn relay_version<T>(response: &http::Response<T>) -> Option<String> {
    let version = response.headers().get("x-starry-version")?.to_str().ok()?;
    validate_version(version)
}

fn validate_version(version: &str) -> Option<String> {
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

fn bounded_header<'a, T>(
    response: &'a http::Response<T>,
    name: &str,
    maximum: usize,
) -> Option<&'a str> {
    let value = response.headers().get(name)?.to_str().ok()?;
    (!value.is_empty() && value.len() <= maximum).then_some(value)
}

fn hex_encode(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode_tag(value: &str) -> Option<auth::Tag> {
    if value.len() != auth::TAGBYTES * 2 {
        return None;
    }
    let mut decoded = vec![0_u8; auth::TAGBYTES];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = hex_nibble(value.as_bytes()[index * 2])?;
        let low = hex_nibble(value.as_bytes()[index * 2 + 1])?;
        *output = (high << 4) | low;
    }
    auth::Tag::from_slice(&decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn header_u64<T>(response: &http::Response<T>, name: &str, maximum: u64) -> Option<u64> {
    let raw = response.headers().get(name)?.to_str().ok()?;
    if raw.is_empty() || raw.len() > 20 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = raw.parse::<u64>().ok()?;
    (value <= maximum).then_some(value)
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
    let observed_now = unix_millis();
    endpoint.last_probe_unix_millis = Some(observed_now);
    match result {
        Ok(success) => {
            if let (Some(previous_instance), Some(current)) = (
                endpoint.telemetry_instance_id.as_deref(),
                success.load.as_ref(),
            ) {
                if previous_instance == current.process_instance_id
                    && (endpoint
                        .telemetry_sequence
                        .map(|sequence| current.sequence <= sequence)
                        .unwrap_or(false)
                        || endpoint
                            .telemetry_uptime_seconds
                            .map(|uptime| current.uptime_seconds < uptime)
                            .unwrap_or(false))
                {
                    let err = ProbeFailure::new(
                        "telemetry_sequence_replay",
                        "telemetry sequence or uptime did not advance monotonically",
                    );
                    apply_failure(endpoint, &err, failure_threshold);
                    log::warn!(
                        "WebSocket Relay probe failed for {relay} ({}): {}",
                        err.code,
                        err.message
                    );
                    state.snapshot_id = next_snapshot_id();
                    return;
                }
                if previous_instance != current.process_instance_id {
                    endpoint.telemetry_restarts = endpoint.telemetry_restarts.saturating_add(1);
                    endpoint.last_restart_unix_millis = Some(observed_now);
                }
            }
            endpoint.consecutive_successes = endpoint.consecutive_successes.saturating_add(1);
            endpoint.consecutive_failures = 0;
            endpoint.last_success_millis = Some(now_millis());
            endpoint.latency_ms = Some(success.latency_ms);
            endpoint.version = success.version;
            endpoint.relay_probe_protocol = success.relay_probe_protocol;
            endpoint.relay_load_protocol = success.relay_load_protocol;
            endpoint.load = (endpoint.relay_probe_protocol.unwrap_or_default() >= 1
                && endpoint.relay_load_protocol.unwrap_or_default() >= 1)
                .then_some(success.load)
                .flatten();
            endpoint.telemetry_observed_unix_millis =
                endpoint.load.as_ref().map(|load| load.observed_at_unix_ms);
            if let Some(load) = endpoint.load.as_ref() {
                endpoint.telemetry_instance_id = Some(load.process_instance_id.clone());
                endpoint.telemetry_sequence = Some(load.sequence);
                endpoint.telemetry_uptime_seconds = Some(load.uptime_seconds);
            }
            endpoint.last_error_code = None;
            endpoint.last_error = None;
            if endpoint.consecutive_successes >= success_threshold {
                endpoint.status = Status::Healthy;
            }
        }
        Err(err) => {
            apply_failure(endpoint, &err, failure_threshold);
            log::warn!(
                "WebSocket Relay probe failed for {relay} ({}): {}",
                err.code,
                err.message
            );
        }
    }
    state.snapshot_id = next_snapshot_id();
}

fn apply_failure(endpoint: &mut EndpointState, err: &ProbeFailure, failure_threshold: u32) {
    endpoint.consecutive_failures = endpoint.consecutive_failures.saturating_add(1);
    endpoint.consecutive_successes = 0;
    endpoint.last_failure_millis = Some(now_millis());
    endpoint.latency_ms = None;
    // Dynamic telemetry is fail-closed. Static inventory metadata remains
    // visible beside the explicit unhealthy/error state.
    endpoint.load = None;
    endpoint.telemetry_observed_unix_millis = None;
    endpoint.last_error_code = Some(err.code);
    endpoint.last_error = Some(err.message.clone());
    if endpoint.consecutive_failures >= failure_threshold {
        endpoint.status = Status::Unhealthy;
    }
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

pub(crate) fn runtime_snapshot(max_telemetry_age_seconds: u64) -> RuntimeHealthSnapshot {
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
            let age_seconds = endpoint
                .telemetry_observed_unix_millis
                .map(|observed| unix_millis().saturating_sub(observed) / 1_000);
            let stale = endpoint.load.is_none()
                || endpoint.relay_probe_protocol.unwrap_or_default() < 1
                || endpoint.relay_load_protocol.unwrap_or_default() < 1
                || age_seconds
                    .map(|age| age > max_telemetry_age_seconds)
                    .unwrap_or(true);
            (
                configured.relay.to_ascii_lowercase(),
                RuntimeEndpointSnapshot {
                    relay: configured.relay.clone(),
                    url: configured.url.clone(),
                    state: status_name(endpoint.status),
                    last_probe_at: endpoint.last_probe_unix_millis.and_then(rfc3339_millis),
                    observed_at: endpoint
                        .telemetry_observed_unix_millis
                        .and_then(rfc3339_millis),
                    observed_at_unix_ms: endpoint.telemetry_observed_unix_millis,
                    age_seconds,
                    stale,
                    latency_ms: endpoint.latency_ms,
                    version: endpoint.version,
                    relay_probe_protocol: endpoint.relay_probe_protocol,
                    relay_load_protocol: endpoint.relay_load_protocol,
                    telemetry_instance_id: endpoint.telemetry_instance_id,
                    telemetry_sequence: endpoint.telemetry_sequence,
                    telemetry_uptime_seconds: endpoint.telemetry_uptime_seconds,
                    load: endpoint.load,
                    telemetry_restarts: endpoint.telemetry_restarts,
                    last_restart_at: endpoint.last_restart_unix_millis.and_then(rfc3339_millis),
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
    let unix_now = unix_millis();
    let max_telemetry_age_seconds = crate::starry_config::snapshot()
        .map(|config| config.relay_quality.max_telemetry_age_seconds)
        .unwrap_or_else(|| {
            crate::starry_config::RelayQualityConfig::default().max_telemetry_age_seconds
        });
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
                relay_probe_protocol: endpoint.relay_probe_protocol,
                relay_load_protocol: endpoint.relay_load_protocol,
                observed_at: endpoint
                    .telemetry_observed_unix_millis
                    .and_then(rfc3339_millis),
                age_seconds: endpoint
                    .telemetry_observed_unix_millis
                    .map(|observed| unix_now.saturating_sub(observed) / 1_000),
                stale: endpoint.load.is_none()
                    || endpoint.relay_probe_protocol.unwrap_or_default() < 1
                    || endpoint.relay_load_protocol.unwrap_or_default() < 1
                    || endpoint
                        .telemetry_observed_unix_millis
                        .map(|observed| {
                            unix_now.saturating_sub(observed) / 1_000 > max_telemetry_age_seconds
                        })
                        .unwrap_or(true),
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    static HEALTH_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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
        let _guard = HEALTH_TEST_LOCK.lock().unwrap();
        let relay = "relay-a.example.com:21117".to_owned();
        let config = RelayHealthConfig {
            success_threshold: 2,
            failure_threshold: 2,
            endpoints: vec![RelayEndpointConfig {
                relay: relay.clone(),
                url: "wss://relay-a.example.com/ws/relay".to_owned(),
                telemetry_secret_file: None,
                fast_media_udp_port: None,
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
            probe_success(10, Some("1.1.16-patch-v1.3.0")),
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
            probe_success(10, Some("1.1.16-patch-v1.3.0")),
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
            probe_success(10, Some("1.1.16-patch-v1.3.0")),
            2,
            2,
        );
        assert_ne!(snapshot_id(), first_probe_snapshot);
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
        assert!(runtime_snapshot(180)
            .endpoint(&relay)
            .unwrap()
            .load
            .is_none());
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
        let sequence = SNAPSHOT_SEQUENCE
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        probe_success_for(
            "test-instance",
            sequence,
            unix_millis(),
            latency_ms,
            version,
        )
    }

    fn probe_success_for(
        instance: &str,
        sequence: u64,
        observed_at_unix_ms: u64,
        latency_ms: u64,
        version: Option<&str>,
    ) -> Result<ProbeSuccess, ProbeFailure> {
        Ok(ProbeSuccess {
            latency_ms,
            version: version.map(str::to_owned),
            relay_probe_protocol: Some(1),
            relay_load_protocol: Some(1),
            load: Some(RelayLoadTelemetry {
                telemetry_schema: 1,
                process_instance_id: instance.to_owned(),
                sequence,
                observed_at_unix_ms,
                uptime_seconds: sequence,
                load_basis_points: 1_000,
                active_sessions: 10,
                pending_pairs: 2,
                capacity_sessions: 100,
                bandwidth_bps: 1_000,
                bandwidth_ema_alpha_basis_points: 2_500,
                capacity_bandwidth_bps: 10_000,
                draining: false,
                admission_open: true,
                admission_rejections: 0,
                probe_malformed: 0,
                probe_unsupported: 0,
                probe_rate_limited: 0,
                probe_successful: 1,
                telemetry_auth_failures: 0,
                ..Default::default()
            }),
        })
    }

    #[test]
    fn relay_version_header_is_bounded_and_validated() {
        let response = http::Response::builder()
            .header("x-starry-version", "1.1.16-patch-v1.3.0")
            .body(())
            .unwrap();
        assert_eq!(
            relay_version(&response).as_deref(),
            Some("1.1.16-patch-v1.3.0")
        );

        let response = http::Response::builder()
            .header("x-starry-version", "invalid version")
            .body(())
            .unwrap();
        assert_eq!(relay_version(&response), None);
    }

    #[test]
    fn authenticated_telemetry_is_verified_bounded_and_fail_closed() {
        sodiumoxide::init().unwrap();
        let context = TelemetryRequestContext {
            nonce: "00112233445566778899aabbccddeeff".to_owned(),
            key: auth::Key::from_slice(&[7_u8; auth::KEYBYTES]).unwrap(),
        };
        let envelope = serde_json::json!({
            "telemetry_schema": 1,
            "process_instance_id": "test-instance-1",
            "sequence": 9,
            "observed_at_unix_ms": unix_millis(),
            "uptime_seconds": 30,
            "version": "1.1.16-patch-v1.3.0",
            "relay_probe_protocol": 1,
            "relay_load_protocol": 1,
            "load_basis_points": 2375,
            "active_sessions": 19,
            "pending_pairs": 3,
            "capacity_sessions": 200,
            "bandwidth_bps": 1234567,
            "bandwidth_ema_alpha_basis_points": 2500,
            "capacity_bandwidth_bps": 1000000000_u64,
            "draining": false,
            "admission_open": true,
            "admission_rejections": 4,
            "probe_malformed": 5,
            "probe_unsupported": 6,
            "probe_rate_limited": 7,
            "probe_successful": 8,
            "telemetry_auth_failures": 9,
        });
        let payload = base64::encode_config(
            serde_json::to_vec(&envelope).unwrap(),
            base64::URL_SAFE_NO_PAD,
        );
        let canonical = format!(
            "starry-telemetry-response-v1\n{}\n{}",
            context.nonce, payload
        );
        let signature = hex_encode(auth::authenticate(canonical.as_bytes(), &context.key).as_ref());
        let response = http::Response::builder()
            .header("x-starry-telemetry", &payload)
            .header("x-starry-telemetry-auth", &signature)
            .body(())
            .unwrap();
        let (_, probe_protocol, load_protocol, telemetry) =
            relay_telemetry(&response, &context).unwrap();
        assert_eq!((probe_protocol, load_protocol), (1, 1));
        assert_eq!(telemetry.pending_pairs, 3);
        assert_eq!(telemetry.bandwidth_bps, 1_234_567);
        assert_eq!(telemetry.admission_rejections, 4);

        let invalid = http::Response::builder()
            .header("x-starry-telemetry", payload)
            .header("x-starry-telemetry-auth", "00".repeat(auth::TAGBYTES))
            .body(())
            .unwrap();
        assert_eq!(
            relay_telemetry(&invalid, &context).unwrap_err().code,
            "telemetry_auth_invalid"
        );
    }

    #[test]
    fn telemetry_v2_exposes_authenticated_fast_media_state_only() {
        sodiumoxide::init().unwrap();
        let context = TelemetryRequestContext {
            nonce: "102132435465768798a9bacbdcedfe0f".to_owned(),
            key: auth::Key::from_slice(&[6_u8; auth::KEYBYTES]).unwrap(),
        };
        // Keep the nested counter object separate so serde_json's recursive
        // macro expansion stays below rustc's default recursion limit.
        let fast_media = serde_json::json!({
            "protocol": 1,
            "enabled": true,
            "healthy": true,
            "udp_port": 22119,
            "active_allocations": 2,
            "active_streams": 1,
            "hello_accepted": 3,
            "cookie_rejected": 4,
            "bind_succeeded": 5,
            "bind_rejected": 6,
            "grant_rejected": 7,
            "role_mismatch": 8,
            "session_mismatch": 9,
            "allocation_mismatch": 10,
            "rebinds": 11,
            "forwarded_packets": 12,
            "forwarded_bytes": 13,
            "dropped_packets": 14,
            "rate_limited": 15,
            "replay_rejected": 16,
            "expired_allocations": 17,
            "listener_failures": 18
        });
        let envelope = serde_json::json!({
            "telemetry_schema": 2,
            "process_instance_id": "test-instance-v2",
            "sequence": 10,
            "observed_at_unix_ms": unix_millis(),
            "uptime_seconds": 60,
            "version": "1.1.16-patch-v1.3.1",
            "relay_probe_protocol": 1,
            "relay_load_protocol": 1,
            "load_basis_points": 100,
            "active_sessions": 1,
            "pending_pairs": 0,
            "capacity_sessions": 100,
            "bandwidth_bps": 1000,
            "bandwidth_ema_alpha_basis_points": 2500,
            "capacity_bandwidth_bps": 1000000,
            "draining": false,
            "admission_open": true,
            "admission_rejections": 0,
            "probe_malformed": 0,
            "probe_unsupported": 0,
            "probe_rate_limited": 0,
            "probe_successful": 1,
            "telemetry_auth_failures": 0,
            "fast_media": fast_media
        });
        let payload = base64::encode_config(
            serde_json::to_vec(&envelope).unwrap(),
            base64::URL_SAFE_NO_PAD,
        );
        let canonical = format!(
            "starry-telemetry-response-v1\n{}\n{}",
            context.nonce, payload
        );
        let signature = hex_encode(auth::authenticate(canonical.as_bytes(), &context.key).as_ref());
        let response = http::Response::builder()
            .header("x-starry-telemetry", payload)
            .header("x-starry-telemetry-auth", signature)
            .body(())
            .unwrap();
        let (_, _, _, telemetry) = relay_telemetry(&response, &context).unwrap();
        assert_eq!(telemetry.telemetry_schema, 2);
        assert_eq!(telemetry.fast_media_relay_udp, Some(1));
        assert_eq!(telemetry.fast_media_udp_port, Some(22119));
        assert_eq!(telemetry.fast_media_active_allocations, Some(2));
        assert_eq!(telemetry.fast_media_active_streams, Some(1));
        assert_eq!(telemetry.fast_media_replay_rejected, Some(16));
    }

    #[test]
    fn capability_headers_are_explicit_and_not_inferred_from_version() {
        let capable = http::Response::builder()
            .header("x-starry-version", "1.1.16-patch-v1.3.0")
            .header("x-starry-relay-probe-protocol", "1")
            .header("x-starry-relay-load-protocol", "1")
            .body(())
            .unwrap();
        assert_eq!(
            capability_version(&capable, "x-starry-relay-probe-protocol"),
            Some(1)
        );
        assert_eq!(
            capability_version(&capable, "x-starry-relay-load-protocol"),
            Some(1)
        );

        let legacy = http::Response::builder()
            .header("x-starry-version", "99.0.0-patch-v99.0.0")
            .body(())
            .unwrap();
        assert_eq!(
            capability_version(&legacy, "x-starry-relay-probe-protocol"),
            None
        );
    }

    #[test]
    fn stale_telemetry_remains_observable_but_is_marked_unusable() {
        let _guard = HEALTH_TEST_LOCK.lock().unwrap();
        let relay = "relay-stale.example.com:21117".to_owned();
        let mut endpoint = EndpointState::default();
        endpoint.status = Status::Healthy;
        endpoint.relay_probe_protocol = Some(1);
        endpoint.relay_load_protocol = Some(1);
        endpoint.telemetry_observed_unix_millis = Some(unix_millis().saturating_sub(10_000));
        endpoint.load = Some(RelayLoadTelemetry {
            telemetry_schema: 1,
            process_instance_id: "stale-instance".to_owned(),
            sequence: 7,
            observed_at_unix_ms: unix_millis().saturating_sub(10_000),
            uptime_seconds: 60,
            load_basis_points: 500,
            active_sessions: 1,
            pending_pairs: 0,
            capacity_sessions: 100,
            bandwidth_bps: 1,
            bandwidth_ema_alpha_basis_points: 2_500,
            capacity_bandwidth_bps: 100,
            draining: false,
            admission_open: true,
            admission_rejections: 0,
            probe_malformed: 0,
            probe_unsupported: 0,
            probe_rate_limited: 0,
            probe_successful: 0,
            telemetry_auth_failures: 0,
            ..Default::default()
        });
        *STATE.write().unwrap() = HealthState {
            enabled: true,
            generation: 77,
            completed_generation: 77,
            snapshot_id: 77,
            config: RelayHealthConfig {
                endpoints: vec![RelayEndpointConfig {
                    relay: relay.clone(),
                    url: "wss://relay-stale.example.com/ws/relay".to_owned(),
                    telemetry_secret_file: None,
                    fast_media_udp_port: None,
                }],
                ..Default::default()
            },
            endpoints: HashMap::from([(relay.to_ascii_lowercase(), endpoint)]),
        };

        let snapshot = runtime_snapshot(5);
        let endpoint = snapshot.endpoint(&relay).unwrap();
        assert!(endpoint.stale);
        assert!(endpoint.age_seconds.unwrap() >= 10);
        assert!(endpoint.load.is_some());
    }

    #[test]
    fn telemetry_sequence_replay_fails_closed_and_instance_restart_is_counted() {
        let _guard = HEALTH_TEST_LOCK.lock().unwrap();
        let relay = "relay-restart.example.com:21117".to_owned();
        *STATE.write().unwrap() = HealthState {
            enabled: true,
            generation: 88,
            completed_generation: 88,
            snapshot_id: 88,
            config: RelayHealthConfig {
                endpoints: vec![RelayEndpointConfig {
                    relay: relay.clone(),
                    url: "wss://relay-restart.example.com/ws/telemetry".to_owned(),
                    telemetry_secret_file: Some("/run/secrets/test".to_owned()),
                    fast_media_udp_port: None,
                }],
                ..Default::default()
            },
            endpoints: HashMap::from([(relay.to_ascii_lowercase(), EndpointState::default())]),
        };

        record(
            88,
            &relay,
            probe_success_for("instance-a", 10, unix_millis(), 5, Some("1.0.0")),
            1,
            1,
        );
        record(
            88,
            &relay,
            probe_success_for("instance-a", 10, unix_millis(), 5, Some("1.0.0")),
            1,
            1,
        );
        let replay = runtime_snapshot(180);
        let replay = replay.endpoint(&relay).unwrap();
        assert_eq!(replay.state, "unhealthy");
        assert_eq!(
            replay.error_code.as_deref(),
            Some("telemetry_sequence_replay")
        );
        assert!(replay.load.is_none());

        record(
            88,
            &relay,
            probe_success_for("instance-b", 1, unix_millis(), 5, Some("1.0.0")),
            1,
            1,
        );
        let restarted = runtime_snapshot(180);
        let restarted = restarted.endpoint(&relay).unwrap();
        assert_eq!(restarted.state, "healthy");
        assert_eq!(
            restarted.telemetry_instance_id.as_deref(),
            Some("instance-b")
        );
        assert_eq!(restarted.telemetry_restarts, 1);
        assert!(restarted.last_restart_at.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn health_probe_fanout_is_concurrent_and_bounded() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let results = collect_bounded(0..24, MAX_CONCURRENT_HEALTH_PROBES, |value| {
            let in_flight = in_flight.clone();
            let peak = peak.clone();
            async move {
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                value
            }
        })
        .await;

        assert_eq!(results.len(), 24);
        assert!(peak.load(Ordering::SeqCst) > 1);
        assert!(peak.load(Ordering::SeqCst) <= MAX_CONCURRENT_HEALTH_PROBES);
    }
}
