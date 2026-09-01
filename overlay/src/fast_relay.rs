use crate::{
    connection_auth::{AuthDecision, SignalTransport},
    relay_quality::RelaySelection,
    starry_config::{self, FastRelayConfig},
};
use hbb_common::{
    bytes::Bytes, log, protobuf::Message as _, rendezvous_proto::FastRelayAuthorization,
};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use sodiumoxide::crypto::sign;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    net::IpAddr,
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
const MAX_SESSION_UUID_BYTES: usize = 128;
const SIGNING_RATE_WINDOW_SECONDS: u64 = 60;
const MAX_SIGNATURES_PER_SOURCE_PER_MINUTE: u32 = 120;

static STATE: Lazy<RwLock<FastRelayState>> = Lazy::new(|| RwLock::new(FastRelayState::default()));

#[derive(Default)]
struct FastRelayState {
    grants: HashMap<GrantKey, GrantRecord>,
    sessions: HashMap<String, GrantKey>,
    responses: HashMap<ResponseKey, GrantKey>,
    signing_rates: HashMap<IpAddr, SigningRate>,
    issued: u64,
    reused: u64,
    delivered: u64,
    disabled: u64,
    insecure_requests: u64,
    invalid_configuration: u64,
    invalid_uuids: u64,
    missing_signing_keys: u64,
    signing_failures: u64,
    quality_selection_failures: u64,
    rate_limited: u64,
    response_misses: u64,
    expired_cache_evictions: u64,
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
    signed: Bytes,
    selected_relay: String,
    quality_allocation_id: Vec<u8>,
    config_generation: u64,
    expires_at: u64,
    created: Instant,
}

struct SigningRate {
    window_started: Instant,
    count: u32,
}

#[derive(Clone)]
struct PolicySnapshot {
    generation: u64,
    config: FastRelayConfig,
    allocation_ttl_seconds: u64,
    max_authorizations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) enabled: bool,
    pub(crate) active_authorizations: usize,
    pub(crate) issued: u64,
    pub(crate) reused: u64,
    pub(crate) delivered: u64,
    pub(crate) disabled: u64,
    pub(crate) insecure_requests: u64,
    pub(crate) invalid_configuration: u64,
    pub(crate) invalid_uuids: u64,
    pub(crate) missing_signing_keys: u64,
    pub(crate) signing_failures: u64,
    pub(crate) quality_selection_failures: u64,
    pub(crate) rate_limited: u64,
    pub(crate) response_misses: u64,
    pub(crate) expired_cache_evictions: u64,
}

pub(crate) fn authorization_for_request(
    session_uuid: &str,
    source_ip: IpAddr,
    transport: SignalTransport,
    auth: &AuthDecision,
    selection: Option<&RelaySelection>,
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
        transport,
        auth,
        selection,
        signing_key,
        now,
    )
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
        return RuntimeSnapshot {
            protocol_version: PROTOCOL_VERSION,
            enabled: policy.config.fast_compat_enabled,
            active_authorizations: 0,
            issued: 0,
            reused: 0,
            delivered: 0,
            disabled: 0,
            insecure_requests: 0,
            invalid_configuration: 0,
            invalid_uuids: 0,
            missing_signing_keys: 0,
            signing_failures: 0,
            quality_selection_failures: 0,
            rate_limited: 0,
            response_misses: 0,
            expired_cache_evictions: 0,
        };
    };
    cleanup(&mut state, &policy, now);
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        enabled: policy.config.fast_compat_enabled,
        active_authorizations: state.grants.len(),
        issued: state.issued,
        reused: state.reused,
        delivered: state.delivered,
        disabled: state.disabled,
        insecure_requests: state.insecure_requests,
        invalid_configuration: state.invalid_configuration,
        invalid_uuids: state.invalid_uuids,
        missing_signing_keys: state.missing_signing_keys,
        signing_failures: state.signing_failures,
        quality_selection_failures: state.quality_selection_failures,
        rate_limited: state.rate_limited,
        response_misses: state.response_misses,
        expired_cache_evictions: state.expired_cache_evictions,
    }
}

#[allow(clippy::too_many_arguments)]
fn authorize_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    session_uuid: &str,
    source_ip: IpAddr,
    transport: SignalTransport,
    auth: &AuthDecision,
    selection: Option<&RelaySelection>,
    signing_key: Option<&sign::SecretKey>,
    now: Option<u64>,
) -> Option<Bytes> {
    if !policy.config.fast_compat_enabled {
        state.disabled = state.disabled.saturating_add(1);
        return None;
    }
    if !(30..=300).contains(&policy.config.authorization_ttl_seconds)
        || !(1_000..=200_000).contains(&policy.config.max_bitrate_kbps)
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
    let Some(selection) = selection else {
        state.quality_selection_failures = state.quality_selection_failures.saturating_add(1);
        return None;
    };
    if selection.config_generation != policy.generation
        || selection.decision.protocol_version != crate::relay_quality::PROTOCOL_VERSION
        || selection.decision.relay_server.is_empty()
        || selection.decision.allocation_id.len() != 16
    {
        state.quality_selection_failures = state.quality_selection_failures.saturating_add(1);
        return None;
    }
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
        target_ip: normalize_ip(selection.target_ip),
    };
    if let Some(existing_key) = state.sessions.get(session_uuid) {
        if existing_key != &key {
            state.invalid_uuids = state.invalid_uuids.saturating_add(1);
            return None;
        }
    }
    if let Some(existing) = state.grants.get(&key) {
        if existing.config_generation == policy.generation
            && existing.quality_allocation_id.as_slice()
                == selection.decision.allocation_id.as_ref()
            && existing
                .selected_relay
                .eq_ignore_ascii_case(&selection.decision.relay_server)
            && existing.expires_at > now
        {
            let signed = existing.signed.clone();
            state.reused = state.reused.saturating_add(1);
            return Some(signed);
        }
    }
    if !consume_signing_rate(state, normalize_ip(source_ip), policy.max_authorizations) {
        state.rate_limited = state.rate_limited.saturating_add(1);
        return None;
    }
    let Some(expires_at) = now.checked_add(policy.config.authorization_ttl_seconds) else {
        state.signing_failures = state.signing_failures.saturating_add(1);
        return None;
    };
    let signed = match build_signed_authorization(
        session_uuid,
        expires_at,
        policy.config.max_bitrate_kbps,
        signing_key,
    ) {
        Ok(signed) => signed,
        Err(()) => {
            state.signing_failures = state.signing_failures.saturating_add(1);
            return None;
        }
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
            signed: signed.clone(),
            selected_relay: selection.decision.relay_server.clone(),
            quality_allocation_id: selection.decision.allocation_id.to_vec(),
            config_generation: policy.generation,
            expires_at,
            created: Instant::now(),
        },
    );
    state.issued = state.issued.saturating_add(1);
    log::info!(
        "FastCompat Relay authorization issued: allocation={} relay={} policy_generation={} ttl_seconds={} max_bitrate_kbps={}",
        allocation_label(selection.decision.allocation_id.as_ref()),
        selection.decision.relay_server,
        policy.generation,
        policy.config.authorization_ttl_seconds,
        policy.config.max_bitrate_kbps,
    );
    Some(signed)
}

fn response_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    session_uuid: &str,
    source_ip: IpAddr,
    selected_relay: &str,
    now: u64,
) -> Option<Bytes> {
    if !policy.config.fast_compat_enabled
        || session_uuid.is_empty()
        || session_uuid.len() > MAX_SESSION_UUID_BYTES
    {
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
        .map(|record| record.signed.clone());
    if result.is_some() {
        state.delivered = state.delivered.saturating_add(1);
    } else {
        state.response_misses = state.response_misses.saturating_add(1);
    }
    result
}

fn build_signed_authorization(
    session_uuid: &str,
    expires_at: u64,
    max_bitrate_kbps: u32,
    signing_key: &sign::SecretKey,
) -> Result<Bytes, ()> {
    if session_uuid.is_empty()
        || session_uuid.len() > MAX_SESSION_UUID_BYTES
        || !(1_000..=200_000).contains(&max_bitrate_kbps)
    {
        return Err(());
    }
    let payload = FastRelayAuthorization {
        version: PROTOCOL_VERSION,
        session_uuid: session_uuid.to_owned(),
        expires_at,
        allow_fast_compat: true,
        allow_fast_media_v1: false,
        max_bitrate_kbps,
        ..Default::default()
    }
    .write_to_bytes()
    .map_err(|_| ())?;
    Ok(sign::sign(&payload, signing_key).into())
}

fn current_policy() -> PolicySnapshot {
    let active = starry_config::active_snapshot();
    let Some(config) = active.config.as_ref() else {
        return PolicySnapshot {
            generation: active.generation,
            config: FastRelayConfig::default(),
            allocation_ttl_seconds: 30,
            max_authorizations: 10_000,
        };
    };
    PolicySnapshot {
        generation: active.generation,
        config: config.fast_mode.relay.clone(),
        allocation_ttl_seconds: config.relay_quality.allocation_ttl_seconds,
        max_authorizations: config.relay_quality.max_allocations,
    }
}

fn cleanup(state: &mut FastRelayState, policy: &PolicySnapshot, now: u64) {
    let before = state.grants.len();
    let ttl = Duration::from_secs(policy.allocation_ttl_seconds);
    state
        .grants
        .retain(|_, record| record.created.elapsed() <= ttl && record.expires_at > now);
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

fn allocation_label(allocation_id: &[u8]) -> String {
    let mut output = String::with_capacity(16);
    for byte in allocation_id.iter().take(8) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starry_config::ConnectionAuthMode;
    use hbb_common::rendezvous_proto::RelayQualityDecision;

    fn policy() -> PolicySnapshot {
        PolicySnapshot {
            generation: 7,
            config: FastRelayConfig {
                fast_compat_enabled: true,
                authorization_ttl_seconds: 90,
                max_bitrate_kbps: 50_000,
            },
            allocation_ttl_seconds: 30,
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
    fn signed_grant_round_trips_and_never_enables_fast_media() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let signed =
            build_signed_authorization("session-1", 1_800_000_090, 50_000, &secret).unwrap();
        let payload = sign::verify(signed.as_ref(), &public).unwrap();
        let grant = FastRelayAuthorization::parse_from_bytes(&payload).unwrap();
        assert_eq!(grant.version, 1);
        assert_eq!(grant.session_uuid, "session-1");
        assert_eq!(grant.expires_at, 1_800_000_090);
        assert!(grant.allow_fast_compat);
        assert!(!grant.allow_fast_media_v1);
        assert_eq!(grant.max_bitrate_kbps, 50_000);

        let mut tampered = signed.to_vec();
        tampered[0] ^= 0x80;
        assert!(sign::verify(&tampered, &public).is_err());
    }

    #[test]
    fn retry_reuses_exact_grant_and_response_is_source_bound() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let policy = policy();
        let selection = selection();
        let source: IpAddr = "192.0.2.10".parse().unwrap();
        let first = authorize_locked(
            &mut state,
            &policy,
            "session-1",
            source,
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection),
            Some(&secret),
            Some(1_800_000_000),
        )
        .unwrap();
        let retry = authorize_locked(
            &mut state,
            &policy,
            "session-1",
            source,
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection),
            Some(&secret),
            Some(1_800_000_001),
        )
        .unwrap();
        assert_eq!(first, retry);
        assert_eq!(state.issued, 1);
        assert_eq!(state.reused, 1);
        assert!(authorize_locked(
            &mut state,
            &policy,
            "session-1",
            "192.0.2.11".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection),
            Some(&secret),
            Some(1_800_000_001),
        )
        .is_none());
        assert_eq!(state.invalid_uuids, 1);
        assert!(response_locked(
            &mut state,
            &policy,
            "session-1",
            selection.target_ip,
            "relay-b.example:21117",
            1_800_000_001,
        )
        .is_none());
        assert!(response_locked(
            &mut state,
            &policy,
            "session-1",
            "203.0.113.99".parse().unwrap(),
            &selection.decision.relay_server,
            1_800_000_001,
        )
        .is_none());
        assert_eq!(
            response_locked(
                &mut state,
                &policy,
                "session-1",
                selection.target_ip,
                &selection.decision.relay_server,
                1_800_000_001,
            )
            .unwrap(),
            first
        );
    }

    #[test]
    fn insecure_or_unselected_requests_fall_back_without_a_grant() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let policy = policy();
        let mut state = FastRelayState::default();
        assert!(authorize_locked(
            &mut state,
            &policy,
            "session-1",
            "192.0.2.10".parse().unwrap(),
            SignalTransport::Tcp,
            &auth(),
            Some(&selection()),
            Some(&secret),
            Some(1_800_000_000),
        )
        .is_none());
        assert!(authorize_locked(
            &mut state,
            &policy,
            "session-2",
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            None,
            Some(&secret),
            Some(1_800_000_000),
        )
        .is_none());
        assert_eq!(state.insecure_requests, 1);
        assert_eq!(state.quality_selection_failures, 1);
        assert!(state.grants.is_empty());
    }

    #[test]
    fn new_signatures_are_rate_limited_per_normalized_source() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let policy = policy();
        let selection = selection();
        let source: IpAddr = "192.0.2.10".parse().unwrap();
        let mut state = FastRelayState::default();

        for index in 0..MAX_SIGNATURES_PER_SOURCE_PER_MINUTE {
            assert!(authorize_locked(
                &mut state,
                &policy,
                &format!("session-{index}"),
                source,
                SignalTransport::SecureTcp,
                &auth(),
                Some(&selection),
                Some(&secret),
                Some(1_800_000_000),
            )
            .is_some());
        }
        assert!(authorize_locked(
            &mut state,
            &policy,
            "session-rate-limited",
            source,
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection),
            Some(&secret),
            Some(1_800_000_000),
        )
        .is_none());
        assert_eq!(
            state.issued,
            u64::from(MAX_SIGNATURES_PER_SOURCE_PER_MINUTE)
        );
        assert_eq!(state.rate_limited, 1);
    }
}
