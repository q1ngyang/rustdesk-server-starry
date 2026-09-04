use crate::{
    connection_auth::{AuthDecision, SignalTransport},
    relay_observer::FastMediaRelayEndpoint,
    relay_quality::RelaySelection,
    starry_config::{self, FastRelayConfig},
};
use hbb_common::{
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::{
        FastMediaRenewalRequest, FastMediaRenewalResponse, FastMediaRenewalStatus,
        FastRelayAuthorization,
    },
};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::sign;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const RENEWAL_PROTOCOL_VERSION: u32 = 1;
pub(crate) const ENDPOINT_CONTROLLER: u32 = 1;
pub(crate) const ENDPOINT_TARGET: u32 = 2;
const MAX_SESSION_UUID_BYTES: usize = 128;
const MAX_RELAY_SERVER_BYTES: usize = 256;
const REQUEST_ID_BYTES: usize = 16;
const AUTHORIZATION_DIGEST_BYTES: usize = 32;
const SIGNING_RATE_WINDOW_SECONDS: u64 = 60;
const MAX_SIGNATURES_PER_SOURCE_PER_MINUTE: u32 = 120;
const MIN_RELAY_DATAGRAM: u32 = 608;
const MAX_RELAY_DATAGRAM: u32 = 1_400;
const RENEWAL_FALLBACK_SAFETY_SECONDS: u64 = 10;
const RENEWAL_RESPONSE_REPLAY_SECONDS: u64 = 30;
const ADMISSION_HEADROOM_BASIS_POINTS: u64 = 9_000;
const BASIS_POINTS: u64 = 10_000;

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
    renewal_requests: u64,
    renewal_succeeded: u64,
    renewal_idempotent_replays: u64,
    renewal_failures: RenewalFailures,
    admission_downcapped: u64,
    admission_rejected: u64,
    last_signed_bitrate_cap_kbps: u32,
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
    relay_allocation_id: Option<Vec<u8>>,
    quality_allocation_id: Option<Vec<u8>>,
    config_generation: u64,
    expires_at: u64,
    created: Instant,
    fast_media: bool,
    controller_route: SocketAddr,
    renewal_enabled: bool,
    relay_session_id: Option<u64>,
    renewal_sequence: u64,
    max_bitrate_kbps: u32,
    relay_udp_protocol: u32,
    relay_udp_port: u32,
    relay_max_datagram: u32,
    wire_bytes_per_second: u64,
    session_deadline_unix: u64,
    pending_renewal: Option<PendingRenewal>,
}

#[derive(Clone)]
struct PendingRenewal {
    request_id: [u8; REQUEST_ID_BYTES],
    request_sha256: [u8; AUTHORIZATION_DIGEST_BYTES],
    previous_sequence: u64,
    previous_controller_digest: [u8; AUTHORIZATION_DIGEST_BYTES],
    previous_target_digest: [u8; AUTHORIZATION_DIGEST_BYTES],
    response: FastMediaRenewalResponse,
}

struct SigningRate {
    window_started: Instant,
    count: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct RenewalFailures {
    pub(crate) disabled: u64,
    pub(crate) unauthenticated: u64,
    pub(crate) not_found: u64,
    pub(crate) binding_mismatch: u64,
    pub(crate) expired: u64,
    pub(crate) too_early: u64,
    pub(crate) rate_limited: u64,
    pub(crate) unavailable: u64,
    pub(crate) invalid: u64,
    pub(crate) signing: u64,
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
    pub(crate) renewal_protocol_version: u32,
    pub(crate) fast_compat_enabled: bool,
    pub(crate) fast_media_v1_enabled: bool,
    pub(crate) active_authorizations: usize,
    pub(crate) active_fast_media_authorizations: usize,
    pub(crate) active_renewable_authorizations: usize,
    pub(crate) last_fast_media_authorization_expires_at_unix: u64,
    pub(crate) minimum_remaining_ttl_seconds: Option<u64>,
    pub(crate) renewals_due_within_30_seconds: usize,
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
    pub(crate) renewal_requests: u64,
    pub(crate) renewal_succeeded: u64,
    pub(crate) renewal_idempotent_replays: u64,
    pub(crate) renewal_failures: RenewalFailures,
    pub(crate) admission_downcapped: u64,
    pub(crate) admission_rejected: u64,
    pub(crate) last_signed_bitrate_cap_kbps: u32,
}

pub(crate) fn enabled() -> bool {
    let policy = current_policy();
    policy.config.fast_compat_enabled || policy.config.fast_media_v1_enabled
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn authorization_for_request(
    session_uuid: &str,
    source_route: SocketAddr,
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
        source_route,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn renewal_response(
    request: FastMediaRenewalRequest,
    source_route: SocketAddr,
    source_ip: IpAddr,
    transport: SignalTransport,
    auth: &AuthDecision,
    endpoint: Option<FastMediaRelayEndpoint>,
    signing_key: Option<&sign::SecretKey>,
) -> FastMediaRenewalResponse {
    let policy = current_policy();
    let now = epoch_seconds().unwrap_or_default();
    let Ok(mut state) = STATE.write() else {
        return bare_renewal_response(
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE,
        );
    };
    cleanup(&mut state, &policy, now);
    renewal_locked(
        &mut state,
        &policy,
        request,
        source_route,
        source_ip,
        transport,
        auth,
        endpoint.as_ref(),
        signing_key,
        now,
    )
}

#[allow(clippy::too_many_arguments)]
fn renewal_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    request: FastMediaRenewalRequest,
    source_route: SocketAddr,
    source_ip: IpAddr,
    transport: SignalTransport,
    auth: &AuthDecision,
    endpoint: Option<&FastMediaRelayEndpoint>,
    signing_key: Option<&sign::SecretKey>,
    now: u64,
) -> FastMediaRenewalResponse {
    state.renewal_requests = state.renewal_requests.saturating_add(1);
    if !policy.config.fast_media_v1_enabled {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_DISABLED,
        );
    }
    if !matches!(
        transport,
        SignalTransport::SecureTcp | SignalTransport::WebSocket
    ) || !auth.proceed
        || auth.verdict != "allow"
    {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAUTHENTICATED,
        );
    }
    if request.protocol_version != RENEWAL_PROTOCOL_VERSION
        || request.requester_role != ENDPOINT_CONTROLLER
        || request.session_uuid.is_empty()
        || request.session_uuid.len() > MAX_SESSION_UUID_BYTES
        || request.relay_allocation_id.len() != 16
        || request.relay_session_id == 0
        || request.controller_authorization_sha256.len() != AUTHORIZATION_DIGEST_BYTES
        || request.target_authorization_sha256.len() != AUTHORIZATION_DIGEST_BYTES
        || request.request_id.len() != REQUEST_ID_BYTES
        || request.relay_server.is_empty()
        || request.relay_server.len() > MAX_RELAY_SERVER_BYTES
        || request.relay_udp_protocol != PROTOCOL_VERSION
        || !(MIN_RELAY_DATAGRAM..=MAX_RELAY_DATAGRAM).contains(&request.relay_max_datagram)
        || !(1_000..=200_000).contains(&request.current_max_bitrate_kbps)
    {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_INVALID,
        );
    }
    let Some(key) = state.sessions.get(&request.session_uuid).cloned() else {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND,
        );
    };
    let controller_digest: [u8; AUTHORIZATION_DIGEST_BYTES] = request
        .controller_authorization_sha256
        .as_ref()
        .try_into()
        .expect("validated controller digest length");
    let target_digest: [u8; AUTHORIZATION_DIGEST_BYTES] = request
        .target_authorization_sha256
        .as_ref()
        .try_into()
        .expect("validated target digest length");
    let request_id: [u8; REQUEST_ID_BYTES] = request
        .request_id
        .as_ref()
        .try_into()
        .expect("validated request id length");
    let request_sha256 = match request.write_to_bytes() {
        Ok(bytes) => authorization_digest(&bytes),
        Err(_) => {
            return renewal_failure(
                state,
                &request,
                FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_INVALID,
            )
        }
    };

    let (record_expires_at, session_deadline_unix, relay_udp_port) = {
        let Some(record) = state.grants.get(&key) else {
            return renewal_failure(
                state,
                &request,
                FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND,
            );
        };
        if let Some(pending) = record.pending_renewal.as_ref() {
            if pending.request_id == request_id
                && pending.request_sha256 == request_sha256
                && pending.previous_sequence == request.current_renewal_sequence
                && pending.previous_controller_digest == controller_digest
                && pending.previous_target_digest == target_digest
                && pending.response.expires_at > now
                && renewal_route_matches(record.controller_route, source_route, transport)
                && key.initiator_ip == normalize_ip(source_ip)
                && record
                    .selected_relay
                    .eq_ignore_ascii_case(&request.relay_server)
                && record.relay_allocation_id.as_deref()
                    == Some(request.relay_allocation_id.as_ref())
                && record.relay_session_id == Some(request.relay_session_id)
            {
                let response = pending.response.clone();
                state.renewal_idempotent_replays =
                    state.renewal_idempotent_replays.saturating_add(1);
                return response;
            }
        }

        let current_controller_digest = authorization_digest(&record.controller_signed);
        let current_target_digest = authorization_digest(&record.target_signed);
        let binding_matches = record.fast_media
            && record.renewal_enabled
            && record.config_generation == policy.generation
            && renewal_route_matches(record.controller_route, source_route, transport)
            && key.initiator_ip == normalize_ip(source_ip)
            && record
                .selected_relay
                .eq_ignore_ascii_case(&request.relay_server)
            && record.relay_allocation_id.as_deref() == Some(request.relay_allocation_id.as_ref())
            && record.relay_udp_protocol == request.relay_udp_protocol
            && record.relay_max_datagram == request.relay_max_datagram
            && record.max_bitrate_kbps == request.current_max_bitrate_kbps
            && record.renewal_sequence == request.current_renewal_sequence
            && current_controller_digest == controller_digest
            && current_target_digest == target_digest
            && record
                .relay_session_id
                .map(|session_id| session_id == request.relay_session_id)
                .unwrap_or(true);
        if !binding_matches {
            return renewal_failure(
                state,
                &request,
                FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH,
            );
        }
        (
            record.expires_at,
            record.session_deadline_unix,
            record.relay_udp_port,
        )
    };
    if record_expires_at <= now {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_EXPIRED,
        );
    }
    let Some(endpoint) = endpoint.filter(|endpoint| {
        endpoint.protocol == PROTOCOL_VERSION
            && endpoint.renewal_protocol == Some(RENEWAL_PROTOCOL_VERSION)
            && u32::from(endpoint.udp_port) == relay_udp_port
    }) else {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE,
        );
    };
    if record_expires_at.saturating_sub(now)
        > renewal_window_seconds(policy.config.authorization_ttl_seconds)
    {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_TOO_EARLY,
        );
    }
    if session_deadline_unix <= now.saturating_add(RENEWAL_FALLBACK_SAFETY_SECONDS) {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_EXPIRED,
        );
    }
    let Some(signing_key) = signing_key else {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE,
        );
    };
    if !consume_signing_rate(state, key.initiator_ip, policy.max_authorizations) {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_RATE_LIMITED,
        );
    }
    let Some(actual_bitrate_kbps) = admitted_source_cap(
        state,
        Some(&key),
        &request.relay_server,
        key.initiator_ip,
        key.target_ip,
        request.current_max_bitrate_kbps,
        endpoint,
    ) else {
        state.admission_rejected = state.admission_rejected.saturating_add(1);
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE,
        );
    };
    if actual_bitrate_kbps < request.current_max_bitrate_kbps {
        state.admission_downcapped = state.admission_downcapped.saturating_add(1);
    }
    let next_sequence = match request.current_renewal_sequence.checked_add(1) {
        Some(sequence) => sequence,
        None => {
            return renewal_failure(
                state,
                &request,
                FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_INVALID,
            )
        }
    };
    let expires_at = now
        .saturating_add(policy.config.authorization_ttl_seconds)
        .min(session_deadline_unix);
    if expires_at <= record_expires_at || expires_at <= now {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_EXPIRED,
        );
    }
    let allocation_id = request.relay_allocation_id.as_ref();
    let controller_signed = match build_signed_authorization(
        &request.session_uuid,
        expires_at,
        &policy.config,
        &request.relay_server,
        Some(endpoint),
        Some(allocation_id),
        ENDPOINT_CONTROLLER,
        actual_bitrate_kbps,
        RENEWAL_PROTOCOL_VERSION,
        request.relay_session_id,
        next_sequence,
        &controller_digest,
        signing_key,
    ) {
        Some(signed) => signed,
        None => {
            return renewal_signing_failure(state, &request);
        }
    };
    let target_signed = match build_signed_authorization(
        &request.session_uuid,
        expires_at,
        &policy.config,
        &request.relay_server,
        Some(endpoint),
        Some(allocation_id),
        ENDPOINT_TARGET,
        actual_bitrate_kbps,
        RENEWAL_PROTOCOL_VERSION,
        request.relay_session_id,
        next_sequence,
        &target_digest,
        signing_key,
    ) {
        Some(signed) => signed,
        None => {
            return renewal_signing_failure(state, &request);
        }
    };
    let renew_after = expires_at.saturating_sub(renewal_window_seconds(
        policy.config.authorization_ttl_seconds,
    ));
    let fallback_before = expires_at.saturating_sub(RENEWAL_FALLBACK_SAFETY_SECONDS);
    let response = FastMediaRenewalResponse {
        protocol_version: RENEWAL_PROTOCOL_VERSION,
        status: FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_OK.into(),
        session_uuid: request.session_uuid.clone(),
        relay_allocation_id: request.relay_allocation_id.clone(),
        relay_session_id: request.relay_session_id,
        renewal_sequence: next_sequence,
        expires_at,
        request_id: request.request_id.clone(),
        controller_authorization: controller_signed.clone(),
        target_authorization: target_signed.clone(),
        relay_server: request.relay_server.clone(),
        relay_udp_protocol: request.relay_udp_protocol,
        relay_max_datagram: request.relay_max_datagram,
        max_bitrate_kbps: actual_bitrate_kbps,
        renew_after,
        fallback_before,
        ..Default::default()
    };
    let Some(record) = state.grants.get_mut(&key) else {
        return renewal_failure(
            state,
            &request,
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND,
        );
    };
    record.controller_signed = controller_signed;
    record.target_signed = target_signed;
    record.relay_session_id = Some(request.relay_session_id);
    record.renewal_sequence = next_sequence;
    record.expires_at = expires_at;
    record.max_bitrate_kbps = actual_bitrate_kbps;
    record.wire_bytes_per_second = wire_bytes_per_second(actual_bitrate_kbps);
    record.pending_renewal = Some(PendingRenewal {
        request_id,
        request_sha256,
        previous_sequence: request.current_renewal_sequence,
        previous_controller_digest: controller_digest,
        previous_target_digest: target_digest,
        response: response.clone(),
    });
    state.renewal_succeeded = state.renewal_succeeded.saturating_add(1);
    state.controller_grants_issued = state.controller_grants_issued.saturating_add(1);
    state.target_grants_issued = state.target_grants_issued.saturating_add(1);
    state.last_signed_bitrate_cap_kbps = actual_bitrate_kbps;
    response
}

fn renewal_window_seconds(ttl_seconds: u64) -> u64 {
    (ttl_seconds / 3).clamp(30, 60)
}

fn authorization_digest(authorization: &[u8]) -> [u8; AUTHORIZATION_DIGEST_BYTES] {
    let digest = Sha256::digest(authorization);
    let mut result = [0_u8; AUTHORIZATION_DIGEST_BYTES];
    result.copy_from_slice(&digest);
    result
}

fn bare_renewal_response(
    request: &FastMediaRenewalRequest,
    status: FastMediaRenewalStatus,
) -> FastMediaRenewalResponse {
    FastMediaRenewalResponse {
        protocol_version: RENEWAL_PROTOCOL_VERSION,
        status: status.into(),
        request_id: (request.request_id.len() == REQUEST_ID_BYTES)
            .then(|| request.request_id.clone())
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn renewal_failure(
    state: &mut FastRelayState,
    request: &FastMediaRenewalRequest,
    status: FastMediaRenewalStatus,
) -> FastMediaRenewalResponse {
    match status {
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_DISABLED => {
            state.renewal_failures.disabled = state.renewal_failures.disabled.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAUTHENTICATED => {
            state.renewal_failures.unauthenticated =
                state.renewal_failures.unauthenticated.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND => {
            state.renewal_failures.not_found = state.renewal_failures.not_found.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH => {
            state.renewal_failures.binding_mismatch =
                state.renewal_failures.binding_mismatch.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_EXPIRED => {
            state.renewal_failures.expired = state.renewal_failures.expired.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_TOO_EARLY => {
            state.renewal_failures.too_early = state.renewal_failures.too_early.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_RATE_LIMITED => {
            state.renewal_failures.rate_limited =
                state.renewal_failures.rate_limited.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE => {
            state.renewal_failures.unavailable =
                state.renewal_failures.unavailable.saturating_add(1)
        }
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_INVALID
        | FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNSPECIFIED
        | FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_OK => {
            state.renewal_failures.invalid = state.renewal_failures.invalid.saturating_add(1)
        }
    }
    bare_renewal_response(request, status)
}

fn renewal_signing_failure(
    state: &mut FastRelayState,
    request: &FastMediaRenewalRequest,
) -> FastMediaRenewalResponse {
    state.renewal_failures.signing = state.renewal_failures.signing.saturating_add(1);
    bare_renewal_response(
        request,
        FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAVAILABLE,
    )
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    let policy = current_policy();
    let now = epoch_seconds().unwrap_or_default();
    let Ok(mut state) = STATE.write() else {
        return empty_runtime_snapshot(&policy);
    };
    cleanup(&mut state, &policy, now);
    let fast_media_records = state.grants.values().filter(|record| record.fast_media);
    let minimum_remaining_ttl_seconds = fast_media_records
        .clone()
        .map(|record| record.expires_at.saturating_sub(now))
        .min();
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        renewal_protocol_version: RENEWAL_PROTOCOL_VERSION,
        fast_compat_enabled: policy.config.fast_compat_enabled,
        fast_media_v1_enabled: policy.config.fast_media_v1_enabled,
        active_authorizations: state.grants.len(),
        active_fast_media_authorizations: state
            .grants
            .values()
            .filter(|record| record.fast_media)
            .count(),
        active_renewable_authorizations: state
            .grants
            .values()
            .filter(|record| record.fast_media && record.renewal_enabled)
            .count(),
        last_fast_media_authorization_expires_at_unix: state
            .grants
            .values()
            .filter(|record| record.fast_media)
            .map(|record| record.expires_at)
            .max()
            .unwrap_or_default(),
        minimum_remaining_ttl_seconds,
        renewals_due_within_30_seconds: state
            .grants
            .values()
            .filter(|record| {
                record.fast_media
                    && record.renewal_enabled
                    && record.expires_at.saturating_sub(now) <= 30
            })
            .count(),
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
        renewal_requests: state.renewal_requests,
        renewal_succeeded: state.renewal_succeeded,
        renewal_idempotent_replays: state.renewal_idempotent_replays,
        renewal_failures: state.renewal_failures.clone(),
        admission_downcapped: state.admission_downcapped,
        admission_rejected: state.admission_rejected,
        last_signed_bitrate_cap_kbps: state.last_signed_bitrate_cap_kbps,
    }
}

fn empty_runtime_snapshot(policy: &PolicySnapshot) -> RuntimeSnapshot {
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        renewal_protocol_version: RENEWAL_PROTOCOL_VERSION,
        fast_compat_enabled: policy.config.fast_compat_enabled,
        fast_media_v1_enabled: policy.config.fast_media_v1_enabled,
        active_authorizations: 0,
        active_fast_media_authorizations: 0,
        active_renewable_authorizations: 0,
        last_fast_media_authorization_expires_at_unix: 0,
        minimum_remaining_ttl_seconds: None,
        renewals_due_within_30_seconds: 0,
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
        renewal_requests: 0,
        renewal_succeeded: 0,
        renewal_idempotent_replays: 0,
        renewal_failures: RenewalFailures::default(),
        admission_downcapped: 0,
        admission_rejected: 0,
        last_signed_bitrate_cap_kbps: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn authorize_locked(
    state: &mut FastRelayState,
    policy: &PolicySnapshot,
    session_uuid: &str,
    source_route: SocketAddr,
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
    if selected_relay.is_empty() || selected_relay.len() > MAX_RELAY_SERVER_BYTES {
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
            && existing.controller_route == source_route
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
    let mut fast_media = policy.config.fast_media_v1_enabled && fast_media_endpoint.is_some();
    if policy.config.fast_media_v1_enabled && !fast_media {
        state.fast_media_unavailable = state.fast_media_unavailable.saturating_add(1);
        state.reliable_fallbacks = state.reliable_fallbacks.saturating_add(1);
    }
    let mut actual_bitrate_kbps = policy.config.max_bitrate_kbps;
    if let (true, Some(endpoint)) = (fast_media, fast_media_endpoint.as_ref()) {
        match admitted_source_cap(
            state,
            None,
            selected_relay,
            key.initiator_ip,
            key.target_ip,
            actual_bitrate_kbps,
            endpoint,
        ) {
            Some(admitted) => {
                if admitted < actual_bitrate_kbps {
                    state.admission_downcapped = state.admission_downcapped.saturating_add(1);
                }
                actual_bitrate_kbps = admitted;
            }
            None => {
                state.admission_rejected = state.admission_rejected.saturating_add(1);
                state.reliable_fallbacks = state.reliable_fallbacks.saturating_add(1);
                fast_media = false;
            }
        }
    }
    if !policy.config.fast_compat_enabled && !fast_media {
        return None;
    }
    let relay_allocation_id = fast_media.then(|| uuid::Uuid::now_v7().as_bytes().to_vec());
    let (target_signed, controller_signed) = if let (true, Some(endpoint), Some(allocation_id)) = (
        fast_media,
        fast_media_endpoint.as_ref(),
        relay_allocation_id.as_deref(),
    ) {
        let renewal_protocol = endpoint.renewal_protocol.unwrap_or_default();
        let target = build_signed_authorization(
            session_uuid,
            expires_at,
            &policy.config,
            selected_relay,
            Some(endpoint),
            Some(allocation_id),
            ENDPOINT_TARGET,
            actual_bitrate_kbps,
            renewal_protocol,
            0,
            0,
            &[],
            signing_key,
        )?;
        let controller = build_signed_authorization(
            session_uuid,
            expires_at,
            &policy.config,
            selected_relay,
            Some(endpoint),
            Some(allocation_id),
            ENDPOINT_CONTROLLER,
            actual_bitrate_kbps,
            renewal_protocol,
            0,
            0,
            &[],
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
            actual_bitrate_kbps,
            0,
            0,
            0,
            &[],
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
            relay_allocation_id: relay_allocation_id.clone(),
            quality_allocation_id,
            config_generation: policy.generation,
            expires_at,
            created: Instant::now(),
            fast_media,
            controller_route: source_route,
            renewal_enabled: fast_media
                && fast_media_endpoint
                    .as_ref()
                    .and_then(|endpoint| endpoint.renewal_protocol)
                    == Some(RENEWAL_PROTOCOL_VERSION),
            relay_session_id: None,
            renewal_sequence: 0,
            max_bitrate_kbps: actual_bitrate_kbps,
            relay_udp_protocol: fast_media_endpoint
                .as_ref()
                .map(|endpoint| endpoint.protocol)
                .unwrap_or_default(),
            relay_udp_port: fast_media_endpoint
                .as_ref()
                .map(|endpoint| u32::from(endpoint.udp_port))
                .unwrap_or_default(),
            relay_max_datagram: fast_media
                .then_some(policy.config.relay_max_datagram)
                .unwrap_or_default(),
            wire_bytes_per_second: fast_media
                .then(|| wire_bytes_per_second(actual_bitrate_kbps))
                .unwrap_or_default(),
            session_deadline_unix: fast_media_endpoint
                .as_ref()
                .and_then(|endpoint| endpoint.max_session_seconds)
                .and_then(|seconds| now.checked_add(seconds))
                .unwrap_or(expires_at),
            pending_renewal: None,
        },
    );
    state.issued_sessions = state.issued_sessions.saturating_add(1);
    state.target_grants_issued = state.target_grants_issued.saturating_add(1);
    state.controller_grants_issued = state.controller_grants_issued.saturating_add(1);
    state.fast_compat_sessions = state.fast_compat_sessions.saturating_add(1);
    if fast_media {
        state.fast_media_sessions = state.fast_media_sessions.saturating_add(1);
        state.last_signed_bitrate_cap_kbps = actual_bitrate_kbps;
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
    max_bitrate_kbps: u32,
    renewal_protocol: u32,
    relay_session_id: u64,
    renewal_sequence: u64,
    previous_authorization_sha256: &[u8],
    signing_key: &sign::SecretKey,
) -> Option<Bytes> {
    let fast_media = endpoint.is_some();
    if session_uuid.is_empty()
        || session_uuid.len() > MAX_SESSION_UUID_BYTES
        || !(1_000..=200_000).contains(&max_bitrate_kbps)
        || selected_relay.is_empty()
        || (fast_media
            && (relay_allocation_id.map(<[u8]>::len) != Some(16)
                || !matches!(endpoint_role, ENDPOINT_CONTROLLER | ENDPOINT_TARGET)))
        || (renewal_protocol == 0
            && (relay_session_id != 0
                || renewal_sequence != 0
                || !previous_authorization_sha256.is_empty()))
        || (renewal_protocol == RENEWAL_PROTOCOL_VERSION
            && ((renewal_sequence == 0
                && (relay_session_id != 0 || !previous_authorization_sha256.is_empty()))
                || (renewal_sequence > 0
                    && (relay_session_id == 0
                        || previous_authorization_sha256.len() != AUTHORIZATION_DIGEST_BYTES))))
        || renewal_protocol > RENEWAL_PROTOCOL_VERSION
    {
        return None;
    }
    let payload = FastRelayAuthorization {
        version: PROTOCOL_VERSION,
        session_uuid: session_uuid.to_owned(),
        expires_at,
        allow_fast_compat: config.fast_compat_enabled || fast_media,
        allow_fast_media_v1: fast_media,
        max_bitrate_kbps,
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
        fast_media_relay_renewal: fast_media.then_some(renewal_protocol).unwrap_or_default(),
        relay_session_id: fast_media.then_some(relay_session_id).unwrap_or_default(),
        renewal_sequence: fast_media.then_some(renewal_sequence).unwrap_or_default(),
        previous_authorization_sha256: fast_media
            .then_some(previous_authorization_sha256)
            .unwrap_or_default()
            .to_vec()
            .into(),
        ..Default::default()
    }
    .write_to_bytes()
    .ok()?;
    Some(sign::sign(&payload, signing_key).into())
}

fn admitted_source_cap(
    state: &FastRelayState,
    exclude: Option<&GrantKey>,
    relay: &str,
    controller_ip: IpAddr,
    target_ip: IpAddr,
    requested_kbps: u32,
    endpoint: &FastMediaRelayEndpoint,
) -> Option<u32> {
    if endpoint.renewal_protocol != Some(RENEWAL_PROTOCOL_VERSION) {
        // A v1.3.1/schema-v2 HBBR retains bootstrap-only behavior. Renewal and
        // capacity-aware issuance require the explicit authenticated v3 data.
        return Some(requested_kbps);
    }
    let per_ip_limit = endpoint.per_ip_bytes_per_second?;
    let global_limit = endpoint.global_bytes_per_second?;
    if per_ip_limit == 0 || global_limit == 0 {
        return None;
    }
    let usable_per_ip = per_ip_limit.saturating_mul(ADMISSION_HEADROOM_BASIS_POINTS) / BASIS_POINTS;
    let usable_global = global_limit.saturating_mul(ADMISSION_HEADROOM_BASIS_POINTS) / BASIS_POINTS;
    let mut local_global = 0_u64;
    let mut local_by_ip: HashMap<IpAddr, u64> = HashMap::new();
    for (key, record) in &state.grants {
        if exclude.is_some_and(|excluded| excluded == key)
            || !record.fast_media
            || !record.renewal_enabled
            || !record.selected_relay.eq_ignore_ascii_case(relay)
        {
            continue;
        }
        local_global = local_global.saturating_add(record.wire_bytes_per_second.saturating_mul(2));
        let controller = local_by_ip.entry(key.initiator_ip).or_default();
        *controller = controller.saturating_add(record.wire_bytes_per_second);
        let target = local_by_ip.entry(key.target_ip).or_default();
        *target = target.saturating_add(record.wire_bytes_per_second);
    }
    let excluded_wire = exclude
        .and_then(|key| state.grants.get(key))
        .filter(|record| record.selected_relay.eq_ignore_ascii_case(relay))
        .map(|record| record.wire_bytes_per_second)
        .unwrap_or_default();
    let observed_global = endpoint
        .reserved_bytes_per_second
        .unwrap_or_default()
        .saturating_sub(excluded_wire.saturating_mul(2));
    let used_global = local_global.max(observed_global);
    let available_global = usable_global.saturating_sub(used_global) / 2;
    let observed_peak = endpoint
        .peak_per_ip_reserved_bytes_per_second
        .unwrap_or_default()
        .saturating_sub(excluded_wire);
    let controller_used = local_by_ip
        .get(&controller_ip)
        .copied()
        .unwrap_or_default()
        .max(observed_peak);
    let target_used = local_by_ip
        .get(&target_ip)
        .copied()
        .unwrap_or_default()
        .max(observed_peak);
    let available_per_ip = if controller_ip == target_ip {
        usable_per_ip.saturating_sub(controller_used) / 2
    } else {
        usable_per_ip
            .saturating_sub(controller_used)
            .min(usable_per_ip.saturating_sub(target_used))
    };
    source_cap_for_wire_budget(requested_kbps, available_global.min(available_per_ip))
}

fn source_cap_for_wire_budget(requested_kbps: u32, wire_budget: u64) -> Option<u32> {
    if requested_kbps < 1_000 || wire_bytes_per_second(1_000) > wire_budget {
        return None;
    }
    let mut low = 1_000_u32;
    let mut high = requested_kbps;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if wire_bytes_per_second(middle) <= wire_budget {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Some(low)
}

fn wire_bytes_per_second(source_kbps: u32) -> u64 {
    let wire_kbps = u64::from(source_kbps)
        .saturating_mul(145)
        .saturating_add(99)
        / 100;
    wire_kbps.saturating_mul(1_000).saturating_add(7) / 8
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
        if record.renewal_enabled {
            record.session_deadline_unix > now
                && record
                    .expires_at
                    .saturating_add(RENEWAL_RESPONSE_REPLAY_SECONDS)
                    > now
        } else {
            record.created.elapsed() <= Duration::from_secs(policy.config.authorization_ttl_seconds)
                && record.expires_at > now
        }
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

fn renewal_route_matches(
    original: SocketAddr,
    current: SocketAddr,
    transport: SignalTransport,
) -> bool {
    match transport {
        SignalTransport::WebSocket => original == current,
        SignalTransport::SecureTcp => normalize_ip(original.ip()) == normalize_ip(current.ip()),
        SignalTransport::Tcp | SignalTransport::UnsupportedUdp => false,
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

    fn controller_route() -> SocketAddr {
        "192.0.2.10:40000".parse().unwrap()
    }

    fn endpoint(renewal: bool) -> FastMediaRelayEndpoint {
        FastMediaRelayEndpoint {
            protocol: PROTOCOL_VERSION,
            udp_port: 22_119,
            renewal_protocol: renewal.then_some(RENEWAL_PROTOCOL_VERSION),
            per_ip_bytes_per_second: renewal.then_some(32 * 1024 * 1024),
            global_bytes_per_second: renewal.then_some(512 * 1024 * 1024),
            reserved_bytes_per_second: renewal.then_some(0),
            peak_per_ip_reserved_bytes_per_second: renewal.then_some(0),
            max_session_seconds: renewal.then_some(43_200),
        }
    }

    fn parse_grant(signed: &[u8], public: &sign::PublicKey) -> FastRelayAuthorization {
        FastRelayAuthorization::parse_from_bytes(&sign::verify(signed, public).unwrap()).unwrap()
    }

    fn renewal_request(
        state: &FastRelayState,
        request_id: u8,
        relay_session_id: u64,
    ) -> FastMediaRenewalRequest {
        let record = state.grants.values().next().unwrap();
        FastMediaRenewalRequest {
            protocol_version: RENEWAL_PROTOCOL_VERSION,
            session_uuid: "session-1".to_owned(),
            relay_allocation_id: record.relay_allocation_id.clone().unwrap().into(),
            relay_session_id,
            current_renewal_sequence: record.renewal_sequence,
            controller_authorization_sha256: authorization_digest(&record.controller_signed)
                .to_vec()
                .into(),
            target_authorization_sha256: authorization_digest(&record.target_signed)
                .to_vec()
                .into(),
            request_id: vec![request_id; REQUEST_ID_BYTES].into(),
            token: "authenticated-controller-token".to_owned(),
            relay_server: record.selected_relay.clone(),
            relay_udp_protocol: record.relay_udp_protocol,
            relay_max_datagram: record.relay_max_datagram,
            current_max_bitrate_kbps: record.max_bitrate_kbps,
            requester_role: ENDPOINT_CONTROLLER,
            ..Default::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn issue_media(
        state: &mut FastRelayState,
        policy: &PolicySnapshot,
        endpoint: FastMediaRelayEndpoint,
        secret: &sign::SecretKey,
        session_uuid: &str,
        route: SocketAddr,
        controller_ip: IpAddr,
        target_ip: IpAddr,
        now: u64,
    ) -> Bytes {
        authorize_locked(
            state,
            policy,
            session_uuid,
            route,
            controller_ip,
            target_ip,
            "relay-a.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            None,
            Some(endpoint),
            Some(secret),
            Some(now),
        )
        .unwrap()
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
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            SignalTransport::WebSocket,
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
        let renewal_policy = policy(true);
        let target = authorize_locked(
            &mut state,
            &renewal_policy,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            SignalTransport::SecureTcp,
            &auth(),
            Some(&selection()),
            Some(endpoint(false)),
            Some(&secret),
            Some(1_800_000_000),
        )
        .unwrap();
        let controller = response_locked(
            &mut state,
            &renewal_policy,
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
            controller_route(),
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
            controller_route(),
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

    #[test]
    fn renewal_is_monotonic_idempotent_and_exactly_bound() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let renewal_policy = policy(true);
        let endpoint = endpoint(true);
        let base = 1_800_000_000;
        let target = issue_media(
            &mut state,
            &renewal_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            base,
        );
        let controller = response_locked(
            &mut state,
            &renewal_policy,
            "session-1",
            "198.51.100.20".parse().unwrap(),
            "relay-a.example:21117",
            base + 1,
        )
        .unwrap();
        for grant in [
            parse_grant(&target, &public),
            parse_grant(&controller, &public),
        ] {
            assert_eq!(grant.fast_media_relay_renewal, RENEWAL_PROTOCOL_VERSION);
            assert_eq!(grant.relay_session_id, 0);
            assert_eq!(grant.renewal_sequence, 0);
            assert!(grant.previous_authorization_sha256.is_empty());
        }

        let request = renewal_request(&state, 1, 77);
        let too_early = renewal_locked(
            &mut state,
            &renewal_policy,
            request.clone(),
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 59,
        );
        assert_eq!(
            too_early.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_TOO_EARLY
        );

        let renewed = renewal_locked(
            &mut state,
            &renewal_policy,
            request.clone(),
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 60,
        );
        assert_eq!(
            renewed.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_OK
        );
        assert_eq!(renewed.renewal_sequence, 1);
        assert_eq!(renewed.expires_at, base + 150);
        assert_eq!(renewed.renew_after, base + 120);
        assert_eq!(renewed.fallback_before, base + 140);
        assert_eq!(renewed.relay_session_id, 77);
        let renewed_controller = parse_grant(&renewed.controller_authorization, &public);
        let renewed_target = parse_grant(&renewed.target_authorization, &public);
        assert_eq!(renewed_controller.relay_endpoint_role, ENDPOINT_CONTROLLER);
        assert_eq!(renewed_target.relay_endpoint_role, ENDPOINT_TARGET);
        assert_eq!(renewed_controller.renewal_sequence, 1);
        assert_eq!(renewed_target.renewal_sequence, 1);
        assert_eq!(
            renewed_controller.previous_authorization_sha256,
            request.controller_authorization_sha256
        );
        assert_eq!(
            renewed_target.previous_authorization_sha256,
            request.target_authorization_sha256
        );

        let replayed = renewal_locked(
            &mut state,
            &renewal_policy,
            request.clone(),
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 61,
        );
        assert_eq!(
            renewed.write_to_bytes().unwrap(),
            replayed.write_to_bytes().unwrap(),
            "a lost response must be recoverable with a byte-identical retry"
        );

        let mut altered_same_id = request.clone();
        altered_same_id.current_max_bitrate_kbps = 49_000;
        let altered_same_id = renewal_locked(
            &mut state,
            &renewal_policy,
            altered_same_id,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 61,
        );
        assert_eq!(
            altered_same_id.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH,
            "a request ID cannot replay a byte-different request"
        );

        let mut conflict = request;
        conflict.request_id = vec![2; REQUEST_ID_BYTES].into();
        let conflict = renewal_locked(
            &mut state,
            &renewal_policy,
            conflict,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 61,
        );
        assert_eq!(
            conflict.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );

        let mut wrong_role = renewal_request(&state, 3, 77);
        wrong_role.requester_role = ENDPOINT_TARGET;
        let wrong_role = renewal_locked(
            &mut state,
            &renewal_policy,
            wrong_role,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_role.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_INVALID
        );

        let wrong_route_request = renewal_request(&state, 4, 77);
        let wrong_route = renewal_locked(
            &mut state,
            &renewal_policy,
            wrong_route_request,
            "192.0.2.10:40001".parse().unwrap(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::WebSocket,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_route.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );

        let mut wrong_allocation = renewal_request(&state, 5, 77);
        let mut allocation_id = wrong_allocation.relay_allocation_id.to_vec();
        allocation_id[0] ^= 0xff;
        wrong_allocation.relay_allocation_id = allocation_id.into();
        let wrong_allocation = renewal_locked(
            &mut state,
            &renewal_policy,
            wrong_allocation,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::WebSocket,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_allocation.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );

        let mut wrong_session = renewal_request(&state, 6, 78);
        let mut digest = wrong_session.controller_authorization_sha256.to_vec();
        digest[0] ^= 0xff;
        wrong_session.controller_authorization_sha256 = digest.into();
        let wrong_session = renewal_locked(
            &mut state,
            &renewal_policy,
            wrong_session,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_session.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );

        let plaintext_request = renewal_request(&state, 7, 77);
        let plaintext = renewal_locked(
            &mut state,
            &renewal_policy,
            plaintext_request,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::Tcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            plaintext.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_UNAUTHENTICATED
        );
        assert_eq!(state.grants.values().next().unwrap().renewal_sequence, 1);
    }

    #[test]
    fn hbbs_restart_fails_renewal_closed_without_creating_authority() {
        let mut empty_state = FastRelayState::default();
        let mut request = FastMediaRenewalRequest {
            protocol_version: RENEWAL_PROTOCOL_VERSION,
            session_uuid: "session-1".to_owned(),
            relay_allocation_id: vec![0x41; 16].into(),
            relay_session_id: 77,
            current_renewal_sequence: 1,
            controller_authorization_sha256: vec![0x22; 32].into(),
            target_authorization_sha256: vec![0x33; 32].into(),
            request_id: vec![0x44; 16].into(),
            token: "authenticated-controller-token".to_owned(),
            relay_server: "relay-a.example:21117".to_owned(),
            relay_udp_protocol: 1,
            relay_max_datagram: 1_200,
            current_max_bitrate_kbps: 40_000,
            requester_role: ENDPOINT_CONTROLLER,
            ..Default::default()
        };
        let response = renewal_locked(
            &mut empty_state,
            &policy(true),
            request.clone(),
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint(true)),
            None,
            1_800_000_120,
        );
        assert_eq!(
            response.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND
        );
        assert!(empty_state.grants.is_empty());

        request.session_uuid = "another-session".to_owned();
        let response = renewal_locked(
            &mut empty_state,
            &policy(true),
            request,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint(true)),
            None,
            1_800_000_121,
        );
        assert_eq!(
            response.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_NOT_FOUND
        );
        assert_eq!(empty_state.renewal_failures.not_found, 2);
    }

    #[test]
    fn native_secure_tcp_renewal_allows_only_same_ip_source_port_rotation() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let renewal_policy = policy(true);
        let endpoint = endpoint(true);
        let base = 1_800_000_000;
        issue_media(
            &mut state,
            &renewal_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            base,
        );

        let request = renewal_request(&state, 1, 77);
        let renewed = renewal_locked(
            &mut state,
            &renewal_policy,
            request,
            "192.0.2.10:40123".parse().unwrap(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 60,
        );
        assert_eq!(
            renewed.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_OK
        );

        let request = renewal_request(&state, 2, 77);
        let wrong_route_ip = renewal_locked(
            &mut state,
            &renewal_policy,
            request,
            "192.0.2.11:40124".parse().unwrap(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_route_ip.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );

        let request = renewal_request(&state, 3, 77);
        let wrong_effective_ip = renewal_locked(
            &mut state,
            &renewal_policy,
            request,
            "192.0.2.10:40125".parse().unwrap(),
            "192.0.2.11".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 120,
        );
        assert_eq!(
            wrong_effective_ip.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_BINDING_MISMATCH
        );
    }

    #[test]
    fn controlled_clock_renews_past_bootstrap_and_creation_limits() {
        sodiumoxide::init().unwrap();
        let (_, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let renewal_policy = policy(true);
        let mut endpoint = endpoint(true);
        endpoint.max_session_seconds = Some(600);
        let base = 1_800_000_000;
        issue_media(
            &mut state,
            &renewal_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            base,
        );

        let mut last_expiry = base + 90;
        for (index, elapsed) in (60_u64..=540).step_by(60).enumerate() {
            let request = renewal_request(&state, index as u8 + 1, 77);
            let response = renewal_locked(
                &mut state,
                &renewal_policy,
                request,
                controller_route(),
                "192.0.2.10".parse().unwrap(),
                SignalTransport::WebSocket,
                &auth(),
                Some(&endpoint),
                Some(&secret),
                base + elapsed,
            );
            assert_eq!(
                response.status.enum_value().unwrap(),
                FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_OK
            );
            assert!(response.expires_at > last_expiry);
            last_expiry = response.expires_at;
        }
        assert_eq!(last_expiry, base + 600);
        assert!(last_expiry > base + 300);

        let request = renewal_request(&state, 20, 77);
        let terminal = renewal_locked(
            &mut state,
            &renewal_policy,
            request,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 590,
        );
        assert_eq!(
            terminal.status.enum_value().unwrap(),
            FastMediaRenewalStatus::FAST_MEDIA_RENEWAL_STATUS_EXPIRED
        );

        let mut long_policy = policy(true);
        long_policy.config.authorization_ttl_seconds = 300;
        let mut long_state = FastRelayState::default();
        issue_media(
            &mut long_state,
            &long_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            base,
        );
        let request = renewal_request(&long_state, 42, 88);
        let response = renewal_locked(
            &mut long_state,
            &long_policy,
            request,
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            SignalTransport::SecureTcp,
            &auth(),
            Some(&endpoint),
            Some(&secret),
            base + 240,
        );
        assert_eq!(response.expires_at, base + 540);
    }

    #[test]
    fn admission_downcaps_high_bitrate_and_same_nat_sessions() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let mut state = FastRelayState::default();
        let mut high_policy = policy(true);
        high_policy.config.max_bitrate_kbps = 200_000;
        let endpoint = endpoint(true);
        let base = 1_800_000_000;
        let first = issue_media(
            &mut state,
            &high_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            controller_route(),
            "192.0.2.10".parse().unwrap(),
            "198.51.100.20".parse().unwrap(),
            base,
        );
        let first = parse_grant(&first, &public);
        assert!(first.max_bitrate_kbps < 200_000);
        assert!(
            wire_bytes_per_second(first.max_bitrate_kbps)
                <= (32 * 1024 * 1024_u64) * ADMISSION_HEADROOM_BASIS_POINTS / BASIS_POINTS
        );

        let mut nat_state = FastRelayState::default();
        let mut nat_policy = policy(true);
        nat_policy.config.max_bitrate_kbps = 50_000;
        let nat_ip: IpAddr = "203.0.113.8".parse().unwrap();
        let first = issue_media(
            &mut nat_state,
            &nat_policy,
            endpoint.clone(),
            &secret,
            "session-1",
            "203.0.113.8:40000".parse().unwrap(),
            nat_ip,
            nat_ip,
            base,
        );
        assert_eq!(parse_grant(&first, &public).max_bitrate_kbps, 50_000);
        let second = issue_media(
            &mut nat_state,
            &nat_policy,
            endpoint,
            &secret,
            "session-2",
            "203.0.113.8:40001".parse().unwrap(),
            nat_ip,
            nat_ip,
            base,
        );
        let second = parse_grant(&second, &public);
        assert!(second.max_bitrate_kbps < 50_000);
        let reserved = nat_state
            .grants
            .values()
            .map(|record| record.wire_bytes_per_second.saturating_mul(2))
            .sum::<u64>();
        assert!(
            reserved <= (32 * 1024 * 1024_u64) * ADMISSION_HEADROOM_BASIS_POINTS / BASIS_POINTS
        );
    }
}
