use crate::{
    connection_auth::{AuthDecision, SignalTransport},
    relay_observer::FastMediaRelayEndpoint,
    relay_quality::RelaySelection,
    starry_config::{self, FastRelayConfig},
};
use hbb_common::{bytes::Bytes, protobuf::Message as _, rendezvous_proto::FastRelayAuthorization};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use sodiumoxide::crypto::sign;
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const ENDPOINT_CONTROLLER: u32 = 1;
pub(crate) const ENDPOINT_TARGET: u32 = 2;
const MAX_SESSION_UUID_BYTES: usize = 128;
const SIGNING_RATE_WINDOW_SECONDS: u64 = 60;
const MAX_SIGNATURES_PER_SOURCE_PER_MINUTE: u32 = 120;
const MIN_RELAY_DATAGRAM: u32 = 608;
const MAX_RELAY_DATAGRAM: u32 = 1_400;

static STATE: Lazy<RwLock<FastRelayState>> = Lazy::new(|| RwLock::new(FastRelayState::default()));

#[derive(Default)]
struct FastRelayState {
    grants: HashMap<GrantKey, GrantRecord>,
    sessions: HashMap<String, GrantKey>,
    responses: HashMap<ResponseKey, GrantKey>,
    signing_rates: HashMap<IpAddr, SigningRate>,
    issued_sessions: u64,
    target_grants_issued: u64,
    controller_grants_issued: u64,
    fast_compat_sessions: u64,
    fast_media_sessions: u64,
    reused: u64,
    delivered: u64,
    disabled: u64,
    insecure_requests: u64,
    invalid_configuration: u64,
    invalid_uuids: u64,
    invalid_server_selection: u64,
    missing_signing_keys: u64,
    signing_failures: u64,
    quality_selection_failures: u64,
    rate_limited: u64,
    response_misses: u64,
    expired_cache_evictions: u64,
    fast_media_unavailable: u64,
    reliable_fallbacks: u64,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct GrantKey {
    session_uuid: String,
    initiator_ip: IpAddr,
    target_ip: IpAddr,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct ResponseKey {
    session_uuid: String,
    target_ip: IpAddr,
}

struct GrantRecord {
    target_signed: Bytes,
    controller_signed: Bytes,
    selected_relay: String,
    quality_allocation_id: Option<Vec<u8>>,
    config_generation: u64,
    expires_at: u64,
    created: Instant,
    fast_media: bool,
}

struct SigningRate {
    window_started: Instant,
    count: u32,
}

#[derive(Clone)]
struct PolicySnapshot {
    generation: u64,
    config: FastRelayConfig,
    max_authorizations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) fast_compat_enabled: bool,
    pub(crate) fast_media_v1_enabled: bool,
    pub(crate) active_authorizations: usize,
    pub(crate) active_fast_media_authorizations: usize,
    pub(crate) last_fast_media_authorization_expires_at_unix: u64,
    pub(crate) issued_sessions: u64,
    pub(crate) target_grants_issued: u64,
    pub(crate) controller_grants_issued: u64,
    pub(crate) fast_compat_sessions: u64,
    pub(crate) fast_media_sessions: u64,
    pub(crate) reused: u64,
    pub(crate) delivered: u64,
    pub(crate) disabled: u64,
    pub(crate) insecure_requests: u64,
    pub(crate) invalid_configuration: u64,
    pub(crate) invalid_uuids: u64,
    pub(crate) invalid_server_selection: u64,
    pub(crate) missing_signing_keys: u64,
    pub(crate) signing_failures: u64,
    pub(crate) quality_selection_failures: u64,
    pub(crate) rate_limited: u64,
    pub(crate) response_misses: u64,
    pub(crate) expired_cache_evictions: u64,
    pub(crate) fast_media_unavailable: u64,
    pub(crate) reliable_fallbacks: u64,
}

pub(crate) fn enabled() -> bool {
    let policy = current_policy();
    policy.config.fast_compat_enabled || policy.config.fast_media_v1_enabled
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn authorization_for_request(
    session_uuid: &str,
    source_ip: IpAddr,
    target_ip: IpAddr,
    selected_relay: &str,
    transport: SignalTransport,
    auth: &AuthDecision,
    quality_selection: Option<&RelaySelection>,
    fast_media_endpoint: Option<FastMediaRelayEndpoint>,
    signing_key: Option<&sign::SecretKey>,
) -> Option<Bytes> {
    let policy = current_policy();
    let mut state = STATE.write().ok()?;
    let now = epoch_seconds();
    cleanup(&mut state, &policy, now.unwrap_or_default());
    authorize_locked(
        &mut state,
        &policy,
        session_uuid,
        source_ip,
        target_ip,
        selected_relay,
        transport,
        auth,
        quality_selection,
        fast_media_endpoint,
        signing_key,
        now,
    )
}

pub(crate) fn selected_relay_for_response(session_uuid: &str, source_ip: IpAddr) -> Option<String> {
    let policy = current_policy();
    let mut state = STATE.write().ok()?;
    let now = epoch_seconds().unwrap_or_default();
    cleanup(&mut state, &policy, now);
    let key = ResponseKey {
        session_uuid: session_uuid.to_owned(),
        target_ip: normalize_ip(source_ip),
    };
    state
        .responses
        .get(&key)
        .and_then(|key| state.grants.get(key))
        .filter(|record| record.config_generation == policy.generation && record.expires_at > now)
        .map(|record| record.selected_relay.clone())
}

pub(crate) fn authorization_for_response(
    session_uuid: &str,
    source_ip: IpAddr,
    selected_relay: &str,
) -> Option<Bytes> {
    let policy = current_policy();
    let mut state = STATE.write().ok()?;
    let now = epoch_seconds().unwrap_or_default();
    cleanup(&mut state, &policy, now);
    response_locked(
        &mut state,
        &policy,
        session_uuid,
        source_ip,
        selected_relay,
        now,
    )
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    let policy = current_policy();
    let now = epoch_seconds().unwrap_or_default();
    let Ok(mut state) = STATE.write() else {
        return empty_runtime_snapshot(&policy);
    };
    cleanup(&mut state, &policy, now);
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        fast_compat_enabled: policy.config.fast_compat_enabled,
        fast_media_v1_enabled: policy.config.fast_media_v1_enabled,
        active_authorizations: state.grants.len(),
        active_fast_media_authorizations: state
            .grants
            .values()
            .filter(|record| record.fast_media)
            .count(),
        last_fast_media_authorization_expires_at_unix: state
            .grants
            .values()
            .filter(|record| record.fast_media)
            .map(|record| record.expires_at)
            .max()
            .unwrap_or_default(),
        issued_sessions: state.issued_sessions,
        target_grants_issued: state.target_grants_issued,
        controller_grants_issued: state.controller_grants_issued,
        fast_compat_sessions: state.fast_compat_sessions,
        fast_media_sessions: state.fast_media_sessions,
        reused: state.reused,
        delivered: state.delivered,
        disabled: state.disabled,
        insecure_requests: state.insecure_requests,
        invalid_configuration: state.invalid_configuration,
        invalid_uuids: state.invalid_uuids,
        invalid_server_selection: state.invalid_server_selection,
        missing_signing_keys: state.missing_signing_keys,
        signing_failures: state.signing_failures,
        quality_selection_failures: state.quality_selection_failures,
        rate_limited: state.rate_limited,
        response_misses: state.response_misses,
        expired_cache_evictions: state.expired_cache_evictions,
        fast_media_unavailable: state.fast_media_unavailable,
        reliable_fallbacks: state.reliable_fallbacks,
    }
}

fn empty_runtime_snapshot(policy: &PolicySnapshot) -> RuntimeSnapshot {
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        fast_compat_enabled: policy.config.fast_compat_enabled,
        fast_media_v1_enabled: policy.config.fast_media_v1_enabled,
        active_authorizations: 0,
        active_fast_media_authorizations: 0,
        last_fast_media_authorization_expires_at_unix: 0,
        issued_sessions: 0,
        target_grants_issued: 0,
        controller_grants_issued: 0,
        fast_compat_sessions: 0,
        fast_media_sessions: 0,
        reused: 0,
        delivered: 0,
        disabled: 0,
        insecure_requests: 0,
        invalid_configuration: 0,
        invalid_uuids: 0,
        invalid_server_selection: 0,
        missing_signing_keys: 0,
        signing_failures: 0,
        quality_selection_failures: 0,
        rate_limited: 0,
        response_misses: 0,
        expired_cache_evictions: 0,
        fast_media_unavailable: 0,
        reliable_fallbacks: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn authorize_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    session_uuid: &str,
    source_ip: IpAddr,
    target_ip: IpAddr,
    selected_relay: &str,
    transport: SignalTransport,
    auth: &AuthDecision,
    quality_selection: Option<&RelaySelection>,
    fast_media_endpoint: Option<FastMediaRelayEndpoint>,
    signing_key: Option<&sign::SecretKey>,
    now: Option<u64>,
) -> Option<Bytes> {
    if !policy.config.fast_compat_enabled && !policy.config.fast_media_v1_enabled {
        state.disabled = state.disabled.saturating_add(1);
        return None;
    }
    if !(30..=300).contains(&policy.config.authorization_ttl_seconds)
        || !(1_000..=200_000).contains(&policy.config.max_bitrate_kbps)
        || !(MIN_RELAY_DATAGRAM..=MAX_RELAY_DATAGRAM).contains(&policy.config.relay_max_datagram)
    {
        state.invalid_configuration = state.invalid_configuration.saturating_add(1);
        return None;
    }
    if !matches!(
        transport,
        SignalTransport::SecureTcp | SignalTransport::WebSocket
    ) || !auth.proceed
        || auth.verdict != "allow"
    {
        state.insecure_requests = state.insecure_requests.saturating_add(1);
        return None;
    }
    if session_uuid.is_empty() || session_uuid.len() > MAX_SESSION_UUID_BYTES {
        state.invalid_uuids = state.invalid_uuids.saturating_add(1);
        return None;
    }
    let selected_relay = selected_relay.trim();
    if selected_relay.is_empty() || selected_relay.len() > 256 {
        state.invalid_server_selection = state.invalid_server_selection.saturating_add(1);
        return None;
    }
    let quality_allocation_id = if let Some(selection) = quality_selection {
        if selection.config_generation != policy.generation
            || selection.decision.protocol_version != crate::relay_quality::PROTOCOL_VERSION
            || !selection
                .decision
                .relay_server
                .eq_ignore_ascii_case(selected_relay)
            || selection.decision.allocation_id.len() != 16
            || normalize_ip(selection.target_ip) != normalize_ip(target_ip)
        {
            state.quality_selection_failures = state.quality_selection_failures.saturating_add(1);
            return None;
        }
        Some(selection.decision.allocation_id.to_vec())
    } else {
        None
    };
    let Some(signing_key) = signing_key else {
        state.missing_signing_keys = state.missing_signing_keys.saturating_add(1);
        return None;
    };
    let Some(now) = now else {
        state.signing_failures = state.signing_failures.saturating_add(1);
        return None;
    };
    let key = GrantKey {
        session_uuid: session_uuid.to_owned(),
        initiator_ip: normalize_ip(source_ip),
        target_ip: normalize_ip(target_ip),
    };
    if let Some(existing_key) = state.sessions.get(session_uuid) {
        if existing_key != &key {
            state.invalid_uuids = state.invalid_uuids.saturating_add(1);
            return None;
        }
    }
    if let Some(existing) = state.grants.get(&key) {
        if existing.config_generation == policy.generation
            && existing.quality_allocation_id == quality_allocation_id
            && existing.selected_relay.eq_ignore_ascii_case(selected_relay)
            && existing.expires_at > now
        {
            let signed = existing.target_signed.clone();
            state.reused = state.reused.saturating_add(1);
            return Some(signed);
        }
    }
    if !consume_signing_rate(state, normalize_ip(source_ip), policy.max_authorizations) {
        state.rate_limited = state.rate_limited.saturating_add(1);
        return None;
    }
    let expires_at = now.checked_add(policy.config.authorization_ttl_seconds)?;
    let fast_media = policy.config.fast_media_v1_enabled && fast_media_endpoint.is_some();
    if policy.config.fast_media_v1_enabled && !fast_media {
        state.fast_media_unavailable = state.fast_media_unavailable.saturating_add(1);
        state.reliable_fallbacks = state.reliable_fallbacks.saturating_add(1);
    }
    if !policy.config.fast_compat_enabled && !fast_media {
        return None;
    }
    let relay_allocation_id = fast_media.then(|| uuid::Uuid::now_v7().as_bytes().to_vec());
    let (target_signed, controller_signed) = if let (true, Some(endpoint), Some(allocation_id)) = (
        fast_media,
        fast_media_endpoint,
        relay_allocation_id.as_deref(),
    ) {
        let target = build_signed_authorization(
            session_uuid,
            expires_at,
            &policy.config,
            selected_relay,
            Some(&endpoint),
            Some(allocation_id),
            ENDPOINT_TARGET,
            signing_key,
        )?;
        let controller = build_signed_authorization(
            session_uuid,
            expires_at,
            &policy.config,
            selected_relay,
            Some(&endpoint),
            Some(allocation_id),
            ENDPOINT_CONTROLLER,
            signing_key,
        )?;
        (target, controller)
    } else {
        let signed = build_signed_authorization(
            session_uuid,
            expires_at,
            &policy.config,
            selected_relay,
            None,
            None,
            0,
            signing_key,
        )?;
        (signed.clone(), signed)
    };
    if !state.grants.contains_key(&key) && state.grants.len() >= policy.max_authorizations.max(1) {
        remove_oldest_grant(state);
    }
    remove_grant(state, &key);
    let response_key = ResponseKey {
        session_uuid: session_uuid.to_owned(),
        target_ip: key.target_ip,
    };
    state.sessions.insert(session_uuid.to_owned(), key.clone());
    state.responses.insert(response_key, key.clone());
    state.grants.insert(
        key,
        GrantRecord {
            target_signed: target_signed.clone(),
            controller_signed,
            selected_relay: selected_relay.to_owned(),
            quality_allocation_id,
            config_generation: policy.generation,
            expires_at,
            created: Instant::now(),
            fast_media,
        },
    );
    state.issued_sessions = state.issued_sessions.saturating_add(1);
    state.target_grants_issued = state.target_grants_issued.saturating_add(1);
    state.controller_grants_issued = state.controller_grants_issued.saturating_add(1);
    state.fast_compat_sessions = state.fast_compat_sessions.saturating_add(1);
    if fast_media {
        state.fast_media_sessions = state.fast_media_sessions.saturating_add(1);
    }
    Some(target_signed)
}

fn response_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    session_uuid: &str,
    source_ip: IpAddr,
    selected_relay: &str,
    now: u64,
) -> Option<Bytes> {
    if session_uuid.is_empty() || session_uuid.len() > MAX_SESSION_UUID_BYTES {
        return None;
    }
    let response_key = ResponseKey {
        session_uuid: session_uuid.to_owned(),
        target_ip: normalize_ip(source_ip),
    };
    let result = state
        .responses
        .get(&response_key)
        .and_then(|key| state.grants.get(key))
        .filter(|record| {
            record.config_generation == policy.generation
                && record.expires_at > now
                && record.selected_relay.eq_ignore_ascii_case(selected_relay)
        })
        .map(|record| record.controller_signed.clone());
    if result.is_some() {
        state.delivered = state.delivered.saturating_add(1);
    } else {
        state.response_misses = state.response_misses.saturating_add(1);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn build_signed_authorization(
    session_uuid: &str,
    expires_at: u64,
    config: &FastRelayConfig,
    selected_relay: &str,
    endpoint: Option<&FastMediaRelayEndpoint>,
    relay_allocation_id: Option<&[u8]>,
    endpoint_role: u32,
    signing_key: &sign::SecretKey,
) -> Option<Bytes> {
    let fast_media = endpoint.is_some();
    if session_uuid.is_empty()
        || session_uuid.len() > MAX_SESSION_UUID_BYTES
        || !(1_000..=200_000).contains(&config.max_bitrate_kbps)
        || selected_relay.is_empty()
        || (fast_media
            && (relay_allocation_id.map(<[u8]>::len) != Some(16)
                || !matches!(endpoint_role, ENDPOINT_CONTROLLER | ENDPOINT_TARGET)))
    {
        return None;
    }
    let payload = FastRelayAuthorization {
        version: PROTOCOL_VERSION,
        session_uuid: session_uuid.to_owned(),
        expires_at,
        allow_fast_compat: config.fast_compat_enabled || fast_media,
        allow_fast_media_v1: fast_media,
        max_bitrate_kbps: config.max_bitrate_kbps,
        relay_udp_protocol: endpoint
            .map(|endpoint| endpoint.protocol)
            .unwrap_or_default(),
        // Tag 8 binds every newly issued grant to HBBS's final Relay choice.
        // Legacy six-field grants remain accepted, and legacy protobuf readers
        // ignore this additive field. UDP-only tags stay zero for FastCompat.
        relay_server: selected_relay.to_owned(),
        relay_udp_port: endpoint
            .map(|endpoint| u32::from(endpoint.udp_port))
            .unwrap_or_default(),
        relay_allocation_id: relay_allocation_id.unwrap_or_default().to_vec().into(),
        relay_max_datagram: fast_media
            .then_some(config.relay_max_datagram)
            .unwrap_or_default(),
        relay_endpoint_role: fast_media.then_some(endpoint_role).unwrap_or_default(),
        ..Default::default()
    }
    .write_to_bytes()
    .ok()?;
    Some(sign::sign(&payload, signing_key).into())
}

fn current_policy() -> PolicySnapshot {
    let active = starry_config::active_snapshot();
    let Some(config) = active.config.as_ref() else {
        return PolicySnapshot {
            generation: active.generation,
            config: FastRelayConfig::default(),
            max_authorizations: 10_000,
        };
    };
    PolicySnapshot {
        generation: active.generation,
        config: config.fast_mode.relay.clone(),
        max_authorizations: config.relay_quality.max_allocations,
    }
}

fn cleanup(state: &mut FastRelayState, policy: &PolicySnapshot, now: u64) {
    let before = state.grants.len();
    state.grants.retain(|_, record| {
        record.created.elapsed() <= Duration::from_secs(policy.config.authorization_ttl_seconds)
            && record.expires_at > now
    });
    state.expired_cache_evictions = state
        .expired_cache_evictions
        .saturating_add(before.saturating_sub(state.grants.len()) as u64);
    let active = state.grants.keys().cloned().collect::<HashSet<_>>();
    state
        .sessions
        .retain(|_, grant_key| active.contains(grant_key));
    state
        .responses
        .retain(|_, grant_key| active.contains(grant_key));
    state.signing_rates.retain(|_, rate| {
        rate.window_started.elapsed() <= Duration::from_secs(SIGNING_RATE_WINDOW_SECONDS)
    });
}

fn consume_signing_rate(state: &mut FastRelayState, source_ip: IpAddr, maximum: usize) -> bool {
    if !state.signing_rates.contains_key(&source_ip) && state.signing_rates.len() >= maximum.max(1)
    {
        remove_oldest_signing_rate(state);
    }
    let rate = state.signing_rates.entry(source_ip).or_insert(SigningRate {
        window_started: Instant::now(),
        count: 0,
    });
    if rate.window_started.elapsed() >= Duration::from_secs(SIGNING_RATE_WINDOW_SECONDS) {
        rate.window_started = Instant::now();
        rate.count = 0;
    }
    if rate.count >= MAX_SIGNATURES_PER_SOURCE_PER_MINUTE {
        return false;
    }
    rate.count = rate.count.saturating_add(1);
    true
}

fn remove_grant(state: &mut FastRelayState, key: &GrantKey) {
    if state.grants.remove(key).is_none() {
        return;
    }
    state.sessions.retain(|_, value| value != key);
    state.responses.retain(|_, value| value != key);
}

fn remove_oldest_grant(state: &mut FastRelayState) {
    let oldest = state
        .grants
        .iter()
        .max_by_key(|(_, record)| record.created.elapsed())
        .map(|(key, _)| key.clone());
    if let Some(oldest) = oldest {
        remove_grant(state, &oldest);
    }
}

fn remove_oldest_signing_rate(state: &mut FastRelayState) {
    let oldest = state
        .signing_rates
        .iter()
        .max_by_key(|(_, rate)| rate.window_started.elapsed())
        .map(|(ip, _)| *ip);
    if let Some(oldest) = oldest {
        state.signing_rates.remove(&oldest);
    }
}

fn epoch_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_secs())
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        IpAddr::V4(ip) => IpAddr::V4(ip),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starry_config::ConnectionAuthMode;
    use hbb_common::rendezvous_proto::RelayQualityDecision;

    fn policy(fast_media: bool) -> PolicySnapshot {
        PolicySnapshot {
            generation: 7,
            config: FastRelayConfig {
                fast_compat_enabled: true,
                fast_media_v1_enabled: fast_media,
                authorization_ttl_seconds: 90,
                max_bitrate_kbps: 50_000,
                relay_max_datagram: 1_200,
            },
            max_authorizations: 500,
        }
    }

    fn auth() -> AuthDecision {
        AuthDecision {
            proceed: true,
            verdict: "allow",
            reason: "allow",
            mode: ConnectionAuthMode::Enforce,
        }
    }

    fn selection() -> RelaySelection {
        RelaySelection {
            decision: RelayQualityDecision {
                protocol_version: 1,
                allocation_id: vec![0x41; 16].into(),
                relay_server: "relay-a.example:21117".to_owned(),
                ..Default::default()
            },
            target_ip: "198.51.100.20".parse().unwrap(),
            config_generation: 7,
        }
    }

    #[test]
    fn fast_compat_works_without_a_quality_allocation_and_binds_server_relay() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let target = authorize_locked(
            &mut state,
            &policy(false),
            "session-1",
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            None,
            None,
            Some(&secret),
            Some(1_800_000_000),
        )
        .unwrap();
        let payload = sign::verify(&target, &public).unwrap();
        let grant = FastRelayAuthorization::parse_from_bytes(&payload).unwrap();
        assert!(grant.allow_fast_compat);
        assert!(!grant.allow_fast_media_v1);
        assert_eq!(grant.relay_server, "relay-a.example:21117");
        assert_eq!(grant.relay_endpoint_role, 0);
    }

    #[test]
    fn fast_media_issues_distinct_controller_and_target_role_grants() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let policy = policy(true);
        let target = authorize_locked(
            &mut state,
            &policy,
            "session-1",
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection()),
            Some(FastMediaRelayEndpoint {
                protocol: 1,
                udp_port: 22119,
            }),
            Some(&secret),
            Some(1_800_000_000),
        )
        .unwrap();
        let controller = response_locked(
            &mut state,
            &policy,
            "session-1",
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            1_800_000_001,
        )
        .unwrap();
        assert_ne!(target, controller);
        let target =
            FastRelayAuthorization::parse_from_bytes(&sign::verify(&target, &public).unwrap())
                .unwrap();
        let controller =
            FastRelayAuthorization::parse_from_bytes(&sign::verify(&controller, &public).unwrap())
                .unwrap();
        assert_eq!(target.relay_endpoint_role, ENDPOINT_TARGET);
        assert_eq!(controller.relay_endpoint_role, ENDPOINT_CONTROLLER);
        assert_eq!(target.relay_allocation_id, controller.relay_allocation_id);
        assert_eq!(target.relay_udp_port, 22119);
        assert_eq!(target.relay_max_datagram, 1_200);
    }

    #[test]
    fn unavailable_fast_media_falls_back_to_reliable_fast_compat() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let signed = authorize_locked(
            &mut state,
            &policy(true),
            "session-1",
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            None,
            None,
            Some(&secret),
            Some(1_800_000_000),
        )
        .unwrap();
        let grant =
            FastRelayAuthorization::parse_from_bytes(&sign::verify(&signed, &public).unwrap())
                .unwrap();
        assert!(grant.allow_fast_compat);
        assert!(!grant.allow_fast_media_v1);
        assert_eq!(state.reliable_fallbacks, 1);
    }

    #[test]
    fn quality_selection_must_match_the_server_selected_relay() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        assert!(authorize_locked(
            &mut state,
            &policy(false),
            "session-1",
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-b.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection()),
            None,
            Some(&secret),
            Some(1_800_000_000),
        )
        .is_none());
        assert_eq!(state.quality_selection_failures, 1);
    }
}
