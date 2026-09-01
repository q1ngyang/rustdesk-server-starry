use crate::{
    allocation_explain::{self, AllocationTrace, MatchedRule},
    fast_relay,
    geo_relay::{self, GeoRelaySelection, GeoRuntimeSnapshot},
    profile_activation, relay_quality, starry_config,
    websocket_signal::{self, RelayRequirement, RuntimeHealthSnapshot},
};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::RwLock,
};

static POOL: Lazy<RwLock<RelayPoolState>> = Lazy::new(|| RwLock::new(RelayPoolState::default()));

#[derive(Clone, Default)]
struct RelayPoolState {
    generation: u64,
    configured: Vec<String>,
    native_online: Vec<String>,
    native_observed_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeRuntimeStatus {
    pub(crate) state: String,
    pub(crate) observed_at: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct WebSocketRuntimeStatus {
    pub(crate) configured: bool,
    pub(crate) url: Option<String>,
    pub(crate) state: String,
    pub(crate) last_probe_at: Option<String>,
    pub(crate) observed_at: Option<String>,
    pub(crate) observed_at_unix_ms: Option<u64>,
    pub(crate) age_seconds: Option<u64>,
    pub(crate) stale: bool,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) telemetry_schema: Option<u32>,
    pub(crate) process_instance_id: Option<String>,
    pub(crate) telemetry_sequence: Option<u64>,
    pub(crate) uptime_seconds: Option<u64>,
    pub(crate) telemetry_restarts: u64,
    pub(crate) last_restart_at: Option<String>,
    pub(crate) load_basis_points: Option<u32>,
    pub(crate) active_sessions: Option<u32>,
    pub(crate) pending_pairs: Option<u32>,
    pub(crate) capacity_sessions: Option<u32>,
    pub(crate) bandwidth_bps: Option<u64>,
    pub(crate) bandwidth_ema_alpha_basis_points: Option<u32>,
    pub(crate) capacity_bandwidth_bps: Option<u64>,
    pub(crate) draining: Option<bool>,
    pub(crate) admission_open: Option<bool>,
    pub(crate) admission_rejections: Option<u64>,
    pub(crate) probe_malformed: Option<u64>,
    pub(crate) probe_unsupported: Option<u64>,
    pub(crate) probe_rate_limited: Option<u64>,
    pub(crate) probe_successful: Option<u64>,
    pub(crate) telemetry_auth_failures: Option<u64>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RelayCapabilities {
    pub(crate) relay_probe_protocol: Option<u32>,
    pub(crate) relay_load_protocol: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RelayRuntimeView {
    pub(crate) id: String,
    pub(crate) version: Option<String>,
    pub(crate) capabilities: RelayCapabilities,
    pub(crate) quality_candidate: bool,
    pub(crate) configured_order: usize,
    pub(crate) native: NativeRuntimeStatus,
    pub(crate) websocket: WebSocketRuntimeStatus,
    pub(crate) eligible_for: Vec<String>,
    pub(crate) referenced_by_rules: Vec<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct RelayRuntimeSnapshot {
    pub(crate) config_generation: u64,
    pub(crate) health_snapshot_id: String,
    pub(crate) relays: Vec<RelayRuntimeView>,
    pub(crate) quality: relay_quality::RuntimeSnapshot,
    pub(crate) fast_relay: fast_relay::RuntimeSnapshot,
    pub(crate) profile_activation: profile_activation::RuntimeSnapshot,
    pub(crate) warning: String,
    #[serde(skip)]
    relay_pool_generation: u64,
    #[serde(skip)]
    geo: GeoRuntimeSnapshot,
}

#[derive(Debug)]
pub(crate) struct SimulationError {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    pub(crate) retryable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationRequest {
    client_a: ClientEndpoint,
    client_b: ClientEndpoint,
    transport: String,
    #[serde(rename = "explain", default = "default_true")]
    _explain: bool,
    #[serde(default)]
    expected_config_generation: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientEndpoint {
    ip: String,
}

impl RelayRuntimeSnapshot {
    pub(crate) fn is_consistent(&self) -> bool {
        self.relay_pool_generation == self.config_generation
    }

    pub(crate) fn eligible_relays(&self, requirement: RelayRequirement) -> Vec<String> {
        self.relays
            .iter()
            .filter(|relay| match requirement {
                RelayRequirement::NativeOnly => relay
                    .eligible_for
                    .iter()
                    .any(|transport| transport == "native"),
                RelayRequirement::WebSocketOnly => relay
                    .eligible_for
                    .iter()
                    .any(|transport| transport == "wss"),
                RelayRequirement::Mixed => relay
                    .eligible_for
                    .iter()
                    .any(|transport| transport == "mixed"),
            })
            .map(|relay| relay.id.clone())
            .collect()
    }

    pub(crate) fn configured_relays(&self) -> Vec<String> {
        self.relays.iter().map(|relay| relay.id.clone()).collect()
    }

    pub(crate) fn exclusion_reasons(
        &self,
        requirement: RelayRequirement,
    ) -> HashMap<String, String> {
        self.relays
            .iter()
            .filter_map(|relay| {
                self.exclusion_reason(&relay.id, requirement)
                    .map(|reason| (relay.id.to_ascii_lowercase(), reason))
            })
            .collect()
    }

    pub(crate) fn exclusion_reason(
        &self,
        relay_id: &str,
        requirement: RelayRequirement,
    ) -> Option<String> {
        let relay = self
            .relays
            .iter()
            .find(|relay| relay.id.eq_ignore_ascii_case(relay_id))?;
        let native = relay.native.state == "online";
        let websocket = relay.websocket.state == "healthy";
        match requirement {
            RelayRequirement::NativeOnly if !native => Some("native_unavailable".to_owned()),
            RelayRequirement::WebSocketOnly if !relay.websocket.configured => {
                Some("wss_not_configured".to_owned())
            }
            RelayRequirement::WebSocketOnly if !websocket => Some("wss_unhealthy".to_owned()),
            RelayRequirement::Mixed if !native => Some("native_unavailable".to_owned()),
            RelayRequirement::Mixed if !relay.websocket.configured => {
                Some("wss_not_configured".to_owned())
            }
            RelayRequirement::Mixed if !websocket => Some("wss_unhealthy".to_owned()),
            _ => None,
        }
    }

    pub(crate) fn select_geo(
        &self,
        client_a: IpAddr,
        client_b: IpAddr,
        eligible_relays: &[String],
        requirement: RelayRequirement,
    ) -> Option<GeoRelaySelection> {
        geo_relay::select_relay_explained_from(
            &self.geo,
            client_a,
            client_b,
            eligible_relays,
            requirement,
        )
    }

    pub(crate) fn quality_candidates(
        &self,
        client_a: IpAddr,
        client_b: IpAddr,
        requirement: RelayRequirement,
        fallback: &str,
        maximum: usize,
    ) -> Vec<RelayRuntimeView> {
        let eligible = self.eligible_relays(requirement);
        let mut ordered = self
            .select_geo(client_a, client_b, &eligible, requirement)
            .map(|selection| selection.candidates)
            .unwrap_or_else(|| eligible.clone());
        bounded_quality_views(&self.relays, &mut ordered, fallback, maximum)
    }
}

fn bounded_quality_views(
    relays: &[RelayRuntimeView],
    ordered: &mut Vec<String>,
    fallback: &str,
    maximum: usize,
) -> Vec<RelayRuntimeView> {
    let fallback_is_quality_candidate = relays
        .iter()
        .find(|view| view.id.eq_ignore_ascii_case(fallback))
        .map(|view| view.quality_candidate)
        .unwrap_or(false);
    if fallback_is_quality_candidate {
        if let Some(index) = ordered
            .iter()
            .position(|relay| relay.eq_ignore_ascii_case(fallback))
        {
            ordered.rotate_left(index);
        }
    }
    ordered
        .iter()
        .filter_map(|relay| {
            relays
                .iter()
                .find(|view| view.id.eq_ignore_ascii_case(relay))
                .filter(|view| view.quality_candidate)
                .cloned()
        })
        .take(maximum)
        .collect()
}

pub(crate) fn update_configured(relays: &[String], generation: u64) {
    if let Ok(mut pool) = POOL.write() {
        pool.configured = relays.to_vec();
        // The upstream HBBS Relay pool initially treats every configured Relay
        // as usable, then replaces that list with the result of its periodic
        // reachability probe.  Mirror that transition atomically so routing
        // through this snapshot does not turn a freshly loaded (or single)
        // Relay pool into an artificial outage.
        pool.native_online = relays.to_vec();
        pool.native_observed_at = Some(now());
        pool.generation = generation;
    }
}

pub(crate) fn update_native_online(relays: &[String]) {
    if let Ok(mut pool) = POOL.write() {
        pool.native_online = relays.to_vec();
        pool.native_observed_at = Some(now());
    }
}

pub(crate) fn snapshot() -> RelayRuntimeSnapshot {
    let active = starry_config::active_snapshot();
    let pool = POOL.read().map(|pool| pool.clone()).unwrap_or_default();
    let max_telemetry_age_seconds = active
        .config
        .as_ref()
        .map(|config| config.relay_quality.max_telemetry_age_seconds)
        .unwrap_or_else(|| starry_config::RelayQualityConfig::default().max_telemetry_age_seconds);
    let health = websocket_signal::health_runtime_snapshot(max_telemetry_age_seconds);
    let geo = geo_relay::runtime_snapshot();
    build_snapshot(active, pool, health, geo)
}

pub(crate) fn simulate(
    params: Value,
    rotation_snapshot: usize,
) -> Result<AllocationTrace, SimulationError> {
    let request: SimulationRequest =
        serde_json::from_value(params).map_err(|err| SimulationError {
            code: "REQUEST_INVALID",
            detail: format!("invalid allocation simulation request: {err}"),
            retryable: false,
        })?;
    let client_a = request
        .client_a
        .ip
        .parse::<IpAddr>()
        .map_err(|_| SimulationError {
            code: "IP_INVALID",
            detail: "client_a.ip is not a valid IPv4 or IPv6 address".to_owned(),
            retryable: false,
        })?;
    let client_b = request
        .client_b
        .ip
        .parse::<IpAddr>()
        .map_err(|_| SimulationError {
            code: "IP_INVALID",
            detail: "client_b.ip is not a valid IPv4 or IPv6 address".to_owned(),
            retryable: false,
        })?;
    let requirement =
        RelayRequirement::parse(Some(&request.transport)).ok_or_else(|| SimulationError {
            code: "TRANSPORT_INVALID",
            detail: "transport must be native, wss, or mixed".to_owned(),
            retryable: false,
        })?;
    let snapshot = snapshot();
    if !snapshot.is_consistent() {
        return Err(SimulationError {
            code: "STARRY_NOT_READY",
            detail: "Relay pool and active configuration generations are not synchronized"
                .to_owned(),
            retryable: true,
        });
    }
    if let Some(expected) = request.expected_config_generation {
        if expected != snapshot.config_generation {
            return Err(SimulationError {
                code: "PLAN_STALE",
                detail: format!(
                    "expected configuration generation {expected}, active generation is {}",
                    snapshot.config_generation
                ),
                retryable: false,
            });
        }
    }
    let configured = snapshot.configured_relays();
    let eligible = snapshot.eligible_relays(requirement);
    let matched_rule = snapshot
        .select_geo(client_a, client_b, &eligible, requirement)
        .map(|selection| MatchedRule {
            name: selection.rule_name,
            index: selection.rule_index,
            direction: selection.direction.to_owned(),
            relay_id: selection.relay,
        });
    let exclusion_reasons = snapshot.exclusion_reasons(requirement);
    Ok(allocation_explain::explain_relay_selection(
        &configured,
        &eligible,
        matched_rule,
        rotation_snapshot,
        snapshot.config_generation,
        snapshot.health_snapshot_id,
        &exclusion_reasons,
    ))
}

fn build_snapshot(
    active: starry_config::ActiveConfigSnapshot,
    pool: RelayPoolState,
    health: RuntimeHealthSnapshot,
    geo: GeoRuntimeSnapshot,
) -> RelayRuntimeSnapshot {
    let observed_at = now();
    let native_observed_at = pool
        .native_observed_at
        .clone()
        .unwrap_or_else(|| observed_at.clone());
    let native_online: HashSet<String> = pool
        .native_online
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let websocket_signal_enabled = active
        .config
        .as_ref()
        .map(|config| config.websocket_signal.enabled)
        .unwrap_or(false);
    let legacy_fallback_relays: HashSet<String> = active
        .config
        .as_ref()
        .map(|config| {
            config
                .relay_quality
                .legacy_fallback_relays
                .iter()
                .map(|relay| relay.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let mut rule_references: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(config) = active.config.as_ref() {
        for rule in &config.geo.rules {
            for relay in &rule.relays {
                rule_references
                    .entry(relay.to_ascii_lowercase())
                    .or_default()
                    .push(rule.name.clone());
            }
        }
    }

    let relays = pool
        .configured
        .iter()
        .enumerate()
        .map(|(configured_order, relay)| {
            let native = native_online.contains(&relay.to_ascii_lowercase());
            let endpoint = health.endpoint(relay);
            let websocket_configured = endpoint.is_some();
            let websocket_healthy = websocket_signal_enabled && health.is_healthy(relay);
            let explicitly_legacy = legacy_fallback_relays.contains(&relay.to_ascii_lowercase());
            let relay_probe_protocol = endpoint.and_then(|endpoint| endpoint.relay_probe_protocol);
            let relay_load_protocol = endpoint.and_then(|endpoint| endpoint.relay_load_protocol);
            let quality_candidate = !explicitly_legacy
                && endpoint
                    .map(|endpoint| {
                        endpoint.state == "healthy"
                            && endpoint.relay_probe_protocol.unwrap_or_default() >= 1
                            && endpoint.relay_load_protocol.unwrap_or_default() >= 1
                            && !endpoint.stale
                            && endpoint.load.is_some()
                    })
                    .unwrap_or(false);
            let websocket_state = if !health.enabled {
                "disabled"
            } else if !health.is_ready() || endpoint.is_none() {
                "unknown"
            } else {
                endpoint.map(|endpoint| endpoint.state).unwrap_or("unknown")
            };
            let mut eligible_for = Vec::new();
            if native {
                eligible_for.push("native".to_owned());
            }
            if websocket_healthy {
                eligible_for.push("wss".to_owned());
            }
            if native && websocket_healthy {
                eligible_for.push("mixed".to_owned());
            }
            RelayRuntimeView {
                id: relay.clone(),
                version: endpoint.and_then(|endpoint| endpoint.version.clone()),
                capabilities: RelayCapabilities {
                    relay_probe_protocol,
                    relay_load_protocol,
                },
                quality_candidate,
                configured_order,
                native: NativeRuntimeStatus {
                    state: if native { "online" } else { "offline" }.to_owned(),
                    observed_at: native_observed_at.clone(),
                },
                websocket: WebSocketRuntimeStatus {
                    configured: websocket_configured,
                    url: endpoint.map(|endpoint| public_probe_url(&endpoint.url)),
                    state: websocket_state.to_owned(),
                    last_probe_at: endpoint.and_then(|endpoint| endpoint.last_probe_at.clone()),
                    observed_at: endpoint.and_then(|endpoint| endpoint.observed_at.clone()),
                    observed_at_unix_ms: endpoint.and_then(|endpoint| endpoint.observed_at_unix_ms),
                    age_seconds: endpoint.and_then(|endpoint| endpoint.age_seconds),
                    stale: endpoint.map(|endpoint| endpoint.stale).unwrap_or(true),
                    latency_ms: endpoint.and_then(|endpoint| endpoint.latency_ms),
                    telemetry_schema: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.telemetry_schema),
                    process_instance_id: endpoint
                        .and_then(|endpoint| endpoint.telemetry_instance_id.clone()),
                    telemetry_sequence: endpoint.and_then(|endpoint| endpoint.telemetry_sequence),
                    uptime_seconds: endpoint.and_then(|endpoint| endpoint.telemetry_uptime_seconds),
                    telemetry_restarts: endpoint
                        .map(|endpoint| endpoint.telemetry_restarts)
                        .unwrap_or_default(),
                    last_restart_at: endpoint.and_then(|endpoint| endpoint.last_restart_at.clone()),
                    load_basis_points: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.load_basis_points),
                    active_sessions: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.active_sessions),
                    pending_pairs: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.pending_pairs),
                    capacity_sessions: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.capacity_sessions),
                    bandwidth_bps: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.bandwidth_bps),
                    bandwidth_ema_alpha_basis_points: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.bandwidth_ema_alpha_basis_points),
                    capacity_bandwidth_bps: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.capacity_bandwidth_bps),
                    draining: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.draining),
                    admission_open: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.admission_open),
                    admission_rejections: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.admission_rejections),
                    probe_malformed: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.probe_malformed),
                    probe_unsupported: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.probe_unsupported),
                    probe_rate_limited: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.probe_rate_limited),
                    probe_successful: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.probe_successful),
                    telemetry_auth_failures: endpoint
                        .and_then(|endpoint| endpoint.load.as_ref())
                        .map(|load| load.telemetry_auth_failures),
                    error_code: endpoint.and_then(|endpoint| endpoint.error_code.clone()),
                    error_message: endpoint.and_then(|endpoint| endpoint.error_message.clone()),
                },
                eligible_for,
                referenced_by_rules: rule_references
                    .remove(&relay.to_ascii_lowercase())
                    .unwrap_or_default(),
            }
        })
        .collect();

    RelayRuntimeSnapshot {
        config_generation: active.generation,
        health_snapshot_id: format!("health-{}", health.snapshot_id),
        relays,
        quality: relay_quality::runtime_snapshot(),
        fast_relay: fast_relay::runtime_snapshot(),
        profile_activation: profile_activation::runtime_snapshot(),
        warning: "Relay probes do not prove a complete two-client remote-control session."
            .to_owned(),
        relay_pool_generation: pool.generation,
        geo,
    }
}

fn public_probe_url(telemetry_url: &str) -> String {
    telemetry_url
        .strip_suffix("/ws/telemetry")
        .map(|prefix| format!("{prefix}/ws/relay"))
        .unwrap_or_else(|| telemetry_url.to_owned())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[test]
    fn inventory_preserves_order_and_explains_native_exclusion() {
        let _guard = TEST_LOCK.lock().unwrap();
        update_configured(&["relay-a".to_owned(), "relay-b".to_owned()], 0);
        update_native_online(&["relay-b".to_owned()]);
        let snapshot = snapshot();
        assert_eq!(snapshot.relays[0].configured_order, 0);
        assert_eq!(snapshot.relays[1].configured_order, 1);
        assert_eq!(
            snapshot.exclusion_reason("relay-a", RelayRequirement::NativeOnly),
            Some("native_unavailable".to_owned())
        );
        assert_eq!(
            snapshot.eligible_relays(RelayRequirement::NativeOnly),
            ["relay-b"]
        );
    }

    fn test_relay(id: &str, quality_candidate: bool) -> RelayRuntimeView {
        RelayRuntimeView {
            id: id.to_owned(),
            version: quality_candidate.then(|| "1.1.16-patch-v1.3.0".to_owned()),
            capabilities: RelayCapabilities {
                relay_probe_protocol: quality_candidate.then_some(1),
                relay_load_protocol: quality_candidate.then_some(1),
            },
            quality_candidate,
            configured_order: 0,
            native: NativeRuntimeStatus {
                state: "online".to_owned(),
                observed_at: "2026-08-31T00:00:00.000Z".to_owned(),
            },
            websocket: WebSocketRuntimeStatus {
                configured: quality_candidate,
                url: quality_candidate.then(|| format!("wss://{id}/ws/relay")),
                state: if quality_candidate {
                    "healthy"
                } else {
                    "unknown"
                }
                .to_owned(),
                last_probe_at: None,
                observed_at: None,
                observed_at_unix_ms: None,
                age_seconds: None,
                stale: !quality_candidate,
                latency_ms: quality_candidate.then_some(10),
                load_basis_points: quality_candidate.then_some(100),
                active_sessions: quality_candidate.then_some(1),
                capacity_sessions: quality_candidate.then_some(100),
                bandwidth_bps: quality_candidate.then_some(1),
                capacity_bandwidth_bps: quality_candidate.then_some(100),
                error_code: None,
                ..Default::default()
            },
            eligible_for: vec!["native".to_owned()],
            referenced_by_rules: Vec::new(),
        }
    }

    #[test]
    fn legacy_geo_entries_do_not_consume_quality_candidate_limit() {
        let relays = vec![
            test_relay("legacy-a:21117", false),
            test_relay("legacy-b:21117", false),
            test_relay("legacy-c:21117", false),
            test_relay("quality-d:21117", true),
            test_relay("quality-e:21117", true),
        ];
        let mut ordered = relays.iter().map(|relay| relay.id.clone()).collect();
        let selected = bounded_quality_views(&relays, &mut ordered, "legacy-a:21117", 3);
        assert_eq!(
            selected
                .iter()
                .map(|relay| relay.id.as_str())
                .collect::<Vec<_>>(),
            ["quality-d:21117", "quality-e:21117"]
        );
    }
}
