//! Relay Reallocation v1 server arbitration.
//!
//! This module is deliberately independent from Relay Quality v1. It reuses
//! only the observer's trusted candidate set and never changes v1 wire/state.

use crate::starry_config::{self, RelayReallocationConfig, RelayReallocationPolicyConfig};
use hbb_common::{
    bytes::Bytes,
    rendezvous_proto::{
        RelayReallocationCandidate, RelayReallocationCandidateSnapshot, RelayReallocationCommit,
        RelayReallocationCommitAck, RelayReallocationPrepare, RelayReallocationReady,
        RelayReallocationRequest, RelayReallocationRollback,
    },
};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const ROLE_CONTROLLER: u32 = 1;
pub(crate) const ROLE_TARGET: u32 = 2;
pub(crate) const STATUS_OK: u32 = 1;
pub(crate) const STATUS_DISABLED: u32 = 2;
pub(crate) const STATUS_UNSUPPORTED: u32 = 3;
pub(crate) const STATUS_UNAUTHENTICATED: u32 = 4;
pub(crate) const STATUS_FORBIDDEN: u32 = 5;
pub(crate) const STATUS_RATE_LIMITED: u32 = 6;
pub(crate) const STATUS_STALE_GENERATION: u32 = 7;
pub(crate) const STATUS_BINDING_MISMATCH: u32 = 8;
pub(crate) const STATUS_NO_CANDIDATE: u32 = 9;
pub(crate) const STATUS_BUSY: u32 = 10;
pub(crate) const STATUS_EXPIRED: u32 = 11;
pub(crate) const STATUS_CONFLICT: u32 = 12;
pub(crate) const STATUS_PEER_REJECTED: u32 = 13;
pub(crate) const STATUS_PEER_TIMEOUT: u32 = 14;
pub(crate) const STATUS_CONNECT_FAILED: u32 = 15;
pub(crate) const STATUS_ROLLED_BACK: u32 = 16;
const ID_BYTES: usize = 16;
const BINDING_BYTES: usize = 32;
const MAX_UUID_BYTES: usize = 128;
const MAX_RELAY_BYTES: usize = 256;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const RESULT_REPLAY_TTL: Duration = Duration::from_secs(120);

static STATE: Lazy<RwLock<State>> = Lazy::new(|| RwLock::new(State::default()));

#[derive(Default)]
struct State {
    sessions: HashMap<String, SessionBinding>,
    active: HashMap<[u8; ID_BYTES], Transaction>,
    by_session: HashMap<String, [u8; ID_BYTES]>,
    completed: HashMap<([u8; ID_BYTES], [u8; BINDING_BYTES]), Completed>,
    per_ip: HashMap<IpAddr, VecDeque<Instant>>,
    per_session: HashMap<String, VecDeque<Instant>>,
    global: VecDeque<Instant>,
    counters: Counters,
}

#[derive(Clone)]
pub(crate) struct SessionBinding {
    pub(crate) session_uuid: String,
    pub(crate) controller_id: String,
    pub(crate) target_id: String,
    pub(crate) controller_route: SocketAddr,
    pub(crate) target_route: SocketAddr,
    pub(crate) controller_ip: IpAddr,
    pub(crate) target_ip: IpAddr,
    pub(crate) controller_websocket: bool,
    pub(crate) target_websocket: bool,
    pub(crate) relay_server: String,
    pub(crate) session_generation: u64,
    pub(crate) config_generation: u64,
    pub(crate) controller_protocol: u32,
    pub(crate) target_protocol: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Prepare,
    Commit,
}

struct Transaction {
    reallocation_id: [u8; ID_BYTES],
    request_id: [u8; ID_BYTES],
    request_digest: [u8; BINDING_BYTES],
    binding: SessionBinding,
    initiator_role: u32,
    new_relay: String,
    new_node_id: String,
    new_generation: u64,
    prepare_token: [u8; ID_BYTES],
    deadline: Instant,
    deadline_unix_ms: u64,
    phase: Phase,
    controller_ready: Option<ReadyRecord>,
    target_ready: Option<ReadyRecord>,
    commit: Option<RelayReallocationCommit>,
    controller_authorization: Bytes,
    target_authorization: Bytes,
    controller_committed: bool,
    target_committed: bool,
}

#[derive(Clone, Eq, PartialEq)]
struct ReadyRecord {
    accepted: bool,
    binding: [u8; BINDING_BYTES],
}

struct Completed {
    at: Instant,
    result: ResultPayload,
}

#[derive(Clone)]
pub(crate) enum ResultPayload {
    Prepare(RelayReallocationPrepare),
    Commit(RelayReallocationCommit),
    Rollback(RelayReallocationRollback),
}

#[derive(Clone, Debug, Default, Serialize)]
struct Counters {
    requested: u64,
    accepted: u64,
    idempotent: u64,
    simultaneous: u64,
    conflicts: u64,
    rate_limited: u64,
    binding_mismatch: u64,
    prepared: u64,
    ready_accepted: u64,
    ready_rejected: u64,
    committed: u64,
    commit_acked: u64,
    rolled_back: u64,
    expired: u64,
    generation_changed: u64,
    peer_timeout: u64,
    connect_failed: u64,
    fastmedia_fenced: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) enabled: bool,
    pub(crate) policy: String,
    pub(crate) active: usize,
    pub(crate) configured_nodes: usize,
    pub(crate) requested: u64,
    pub(crate) accepted: u64,
    pub(crate) idempotent: u64,
    pub(crate) simultaneous: u64,
    pub(crate) conflicts: u64,
    pub(crate) rate_limited: u64,
    pub(crate) binding_mismatch: u64,
    pub(crate) prepared: u64,
    pub(crate) ready_accepted: u64,
    pub(crate) ready_rejected: u64,
    pub(crate) committed: u64,
    pub(crate) commit_acked: u64,
    pub(crate) rolled_back: u64,
    pub(crate) expired: u64,
    pub(crate) generation_changed: u64,
    pub(crate) peer_timeout: u64,
    pub(crate) connect_failed: u64,
    pub(crate) fastmedia_fenced: u64,
}

#[derive(Clone)]
pub(crate) struct Dispatch {
    pub(crate) controller_route: SocketAddr,
    pub(crate) target_route: SocketAddr,
    pub(crate) controller_id: String,
    pub(crate) target_id: String,
    pub(crate) controller_websocket: bool,
    pub(crate) target_websocket: bool,
    pub(crate) controller_ip: IpAddr,
    pub(crate) target_ip: IpAddr,
    pub(crate) payload: ResultPayload,
    pub(crate) snapshot: Option<RelayReallocationCandidateSnapshot>,
}

pub(crate) fn register_session(binding: SessionBinding) {
    if binding.session_uuid.is_empty() || binding.session_uuid.len() > MAX_UUID_BYTES {
        return;
    }
    if let Ok(mut state) = STATE.write() {
        state.sessions.insert(binding.session_uuid.clone(), binding);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn register_request(
    uuid: &str,
    controller_route: SocketAddr,
    controller_ip: IpAddr,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_id: &str,
    controller_websocket: bool,
    target_websocket: bool,
    relay_server: &str,
    controller_protocol: u32,
) {
    let generation = starry_config::active_snapshot().generation;
    register_session(SessionBinding {
        session_uuid: uuid.to_owned(),
        controller_id: String::new(),
        target_id: target_id.to_owned(),
        controller_route,
        target_route,
        controller_ip,
        target_ip,
        controller_websocket,
        target_websocket,
        relay_server: relay_server.to_owned(),
        session_generation: 1,
        config_generation: generation,
        controller_protocol,
        target_protocol: 0,
    });
}

pub(crate) fn register_response(
    uuid: &str,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_protocol: u32,
) {
    if let Ok(mut state) = STATE.write() {
        if let Some(binding) = state.sessions.get_mut(uuid) {
            if normalize_ip(binding.target_ip) == normalize_ip(target_ip) {
                binding.target_route = target_route;
                binding.target_protocol = target_protocol;
            }
        }
    }
}

pub(crate) fn remove_session_if_generation(uuid: &str, generation: u64) {
    if let Ok(mut state) = STATE.write() {
        if state
            .sessions
            .get(uuid)
            .is_some_and(|s| s.session_generation == generation)
        {
            state.sessions.remove(uuid);
        }
    }
}

pub(crate) fn candidate_snapshot(
    request_id: &[u8],
    selected_node: Option<&str>,
) -> RelayReallocationCandidateSnapshot {
    let active = starry_config::active_snapshot();
    let Some(config) = active.config.as_ref() else {
        return snapshot_error(request_id, STATUS_DISABLED);
    };
    if !config.relay_reallocation.enabled {
        return snapshot_error(request_id, STATUS_DISABLED);
    }
    let health = crate::websocket_signal::health_runtime_snapshot(
        config.relay_quality.max_telemetry_age_seconds,
    );
    let candidates = config
        .websocket_signal
        .relay_health
        .endpoints
        .iter()
        .filter_map(|endpoint| {
            let node_id = endpoint.node_id.as_ref()?;
            let runtime = health.endpoint(&endpoint.relay);
            let available = runtime.is_some_and(|r| r.state == "healthy" && !r.stale);
            Some(RelayReallocationCandidate {
                node_id: node_id.clone(),
                display_name: endpoint.display_name.clone().unwrap_or_default(),
                region: endpoint.region.clone().unwrap_or_default(),
                availability: if available { 1 } else { 3 },
                quality_band: if available { 4 } else { 4 },
                normalized_score: 0,
                selected: selected_node.is_some_and(|v| v == node_id),
                ..Default::default()
            })
        })
        .take(config.relay_reallocation.max_candidates)
        .collect();
    RelayReallocationCandidateSnapshot {
        protocol_version: PROTOCOL_VERSION,
        request_id: Bytes::copy_from_slice(request_id),
        status: STATUS_OK,
        config_generation: active.generation,
        expires_at_unix_ms: unix_ms()
            .saturating_add(u64::from(config.relay_reallocation.probe_timeout_ms)),
        candidates,
        ..Default::default()
    }
}

pub(crate) fn begin(
    request: RelayReallocationRequest,
    route: SocketAddr,
    ip: IpAddr,
    authenticated: bool,
) -> Result<Dispatch, RelayReallocationRollback> {
    begin_at(request, route, ip, authenticated, Instant::now(), unix_ms())
}

fn begin_at(
    request: RelayReallocationRequest,
    route: SocketAddr,
    ip: IpAddr,
    authenticated: bool,
    now: Instant,
    now_ms: u64,
) -> Result<Dispatch, RelayReallocationRollback> {
    let active = starry_config::active_snapshot();
    let Some(config) = active.config.as_ref() else {
        return Err(rollback_for(&request, STATUS_DISABLED, "", 0, 0));
    };
    let policy = &config.relay_reallocation;
    if !policy.enabled {
        return Err(rollback_for(
            &request,
            STATUS_DISABLED,
            "",
            0,
            active.generation,
        ));
    }
    if !authenticated {
        return Err(rollback_for(
            &request,
            STATUS_UNAUTHENTICATED,
            "",
            0,
            active.generation,
        ));
    }
    let digest = request_digest(&request)
        .ok_or_else(|| rollback_for(&request, STATUS_BINDING_MISMATCH, "", 0, active.generation))?;
    let request_id: [u8; ID_BYTES] =
        request.request_id.as_ref().try_into().map_err(|_| {
            rollback_for(&request, STATUS_BINDING_MISMATCH, "", 0, active.generation)
        })?;
    let mut state = STATE
        .write()
        .map_err(|_| rollback_for(&request, STATUS_BUSY, "", 0, active.generation))?;
    cleanup(&mut state, policy, now);
    state.counters.requested = state.counters.requested.saturating_add(1);
    if let Some(result) = state
        .completed
        .get(&(request_id, digest))
        .map(|done| done.result.clone())
    {
        state.counters.idempotent = state.counters.idempotent.saturating_add(1);
        return match result {
            ResultPayload::Prepare(p) => Ok(dispatch_for(
                state.sessions.get(&request.session_uuid).unwrap(),
                ResultPayload::Prepare(p),
            )),
            ResultPayload::Commit(c) => Ok(dispatch_for(
                state.sessions.get(&request.session_uuid).unwrap(),
                ResultPayload::Commit(c),
            )),
            ResultPayload::Rollback(r) => Err(r),
        };
    }
    let Some(binding) = state.sessions.get(&request.session_uuid).cloned() else {
        state.counters.binding_mismatch += 1;
        return Err(rollback_for(
            &request,
            STATUS_BINDING_MISMATCH,
            "",
            0,
            active.generation,
        ));
    };
    let expected_route = if request.endpoint_role == ROLE_CONTROLLER {
        binding.controller_route
    } else {
        binding.target_route
    };
    let expected_ip = if request.endpoint_role == ROLE_CONTROLLER {
        binding.controller_ip
    } else {
        binding.target_ip
    };
    if !matches!(request.endpoint_role, ROLE_CONTROLLER | ROLE_TARGET)
        || request.protocol_version != PROTOCOL_VERSION
        || request.config_generation != active.generation
        || binding.config_generation != active.generation
        || request.current_session_generation != binding.session_generation
        || !binding
            .relay_server
            .eq_ignore_ascii_case(&request.current_relay_server)
        || normalize_ip(ip) != normalize_ip(expected_ip)
        || normalize_route(route) != normalize_route(expected_route)
        || request.deadline_unix_ms <= now_ms
        || request.deadline_unix_ms > now_ms.saturating_add(u64::from(policy.total_timeout_ms))
        || (request.endpoint_role == ROLE_CONTROLLER
            && binding.controller_protocol != PROTOCOL_VERSION)
        || (request.endpoint_role == ROLE_TARGET && binding.target_protocol != PROTOCOL_VERSION)
    {
        state.counters.binding_mismatch += 1;
        return Err(rollback_for(
            &request,
            if request.config_generation != active.generation {
                STATUS_STALE_GENERATION
            } else {
                STATUS_BINDING_MISMATCH
            },
            &binding.relay_server,
            binding.session_generation,
            active.generation,
        ));
    }
    if !consume_rate(
        &mut state,
        &request.session_uuid,
        normalize_ip(ip),
        policy,
        now,
    ) {
        state.counters.rate_limited += 1;
        return Err(rollback_for(
            &request,
            STATUS_RATE_LIMITED,
            &binding.relay_server,
            binding.session_generation,
            active.generation,
        ));
    }
    if let Some(existing_id) = state.by_session.get(&request.session_uuid).copied() {
        let (
            existing_request_id,
            existing_digest,
            existing_deadline,
            existing_role,
            existing_prepare,
        ) = {
            let existing = state.active.get(&existing_id).expect("indexed transaction");
            (
                existing.request_id,
                existing.request_digest,
                existing.deadline_unix_ms,
                existing.initiator_role,
                prepare_for(existing, request.endpoint_role),
            )
        };
        if existing_request_id == request_id && existing_digest == digest {
            state.counters.idempotent += 1;
            return Ok(dispatch_for(
                &binding,
                ResultPayload::Prepare(existing_prepare),
            ));
        }
        state.counters.simultaneous += 1;
        let incoming_wins = conflict_key(&request)
            < (
                existing_deadline,
                role_priority(existing_role),
                existing_request_id,
            );
        if !incoming_wins {
            state.counters.conflicts += 1;
            return Err(rollback_for(
                &request,
                STATUS_BUSY,
                &binding.relay_server,
                binding.session_generation,
                active.generation,
            ));
        }
        state.active.remove(&existing_id);
    }
    if state.active.len() >= policy.max_active {
        return Err(rollback_for(
            &request,
            STATUS_BUSY,
            &binding.relay_server,
            binding.session_generation,
            active.generation,
        ));
    }
    let Some((new_node_id, new_relay)) = choose_relay(config, &request, &binding.relay_server)
    else {
        return Err(rollback_for(
            &request,
            STATUS_NO_CANDIDATE,
            &binding.relay_server,
            binding.session_generation,
            active.generation,
        ));
    };
    let reallocation_id = fresh_id();
    let prepare_token = fresh_id();
    let txn = Transaction {
        reallocation_id,
        request_id,
        request_digest: digest,
        binding: binding.clone(),
        initiator_role: request.endpoint_role,
        new_relay,
        new_node_id,
        new_generation: binding.session_generation.saturating_add(1),
        prepare_token,
        deadline: now + Duration::from_millis(u64::from(policy.total_timeout_ms)),
        deadline_unix_ms: now_ms.saturating_add(u64::from(policy.total_timeout_ms)),
        phase: Phase::Prepare,
        controller_ready: None,
        target_ready: None,
        commit: None,
        controller_authorization: Bytes::new(),
        target_authorization: Bytes::new(),
        controller_committed: false,
        target_committed: false,
    };
    let prepare = prepare_for(&txn, request.endpoint_role);
    state
        .by_session
        .insert(request.session_uuid.clone(), reallocation_id);
    state.active.insert(reallocation_id, txn);
    state.counters.accepted += 1;
    state.counters.prepared += 1;
    state.counters.fastmedia_fenced += 1;
    let mut dispatch = dispatch_for(&binding, ResultPayload::Prepare(prepare));
    dispatch.snapshot = Some(candidate_snapshot(
        &request.request_id,
        Some(&state.active[&reallocation_id].new_node_id),
    ));
    Ok(dispatch)
}

pub(crate) fn ready(
    message: RelayReallocationReady,
    route: SocketAddr,
    ip: IpAddr,
) -> Option<Dispatch> {
    let id: [u8; ID_BYTES] = message.reallocation_id.as_ref().try_into().ok()?;
    let binding_hash: [u8; BINDING_BYTES] =
        message.new_path_binding_sha256.as_ref().try_into().ok()?;
    let mut state = STATE.write().ok()?;
    let txn = state.active.get_mut(&id)?;
    if txn.phase != Phase::Prepare
        || message.protocol_version != PROTOCOL_VERSION
        || message.request_id.as_ref() != txn.request_id
        || message.session_uuid != txn.binding.session_uuid
        || message.old_session_generation != txn.binding.session_generation
        || message.new_session_generation != txn.new_generation
        || message.config_generation != txn.binding.config_generation
        || message.prepare_token.as_ref() != txn.prepare_token
    {
        state.counters.binding_mismatch += 1;
        return None;
    }
    let (expected_route, expected_ip, slot) = match message.endpoint_role {
        ROLE_CONTROLLER => (
            txn.binding.controller_route,
            txn.binding.controller_ip,
            &mut txn.controller_ready,
        ),
        ROLE_TARGET => (
            txn.binding.target_route,
            txn.binding.target_ip,
            &mut txn.target_ready,
        ),
        _ => return None,
    };
    if normalize_route(route) != normalize_route(expected_route)
        || normalize_ip(ip) != normalize_ip(expected_ip)
    {
        state.counters.binding_mismatch += 1;
        return None;
    }
    let next = ReadyRecord {
        accepted: message.accepted,
        binding: binding_hash,
    };
    if slot.as_ref().is_some_and(|old| old != &next) {
        state.counters.conflicts += 1;
        return None;
    }
    *slot = Some(next);
    let accepted = message.accepted;
    if txn.controller_ready.as_ref().is_some_and(|v| !v.accepted)
        || txn.target_ready.as_ref().is_some_and(|v| !v.accepted)
    {
        return Some(rollback_active(&mut state, id, STATUS_PEER_REJECTED));
    }
    let (Some(controller), Some(target)) = (&txn.controller_ready, &txn.target_ready) else {
        return None;
    };
    if controller.binding != target.binding {
        return Some(rollback_active(&mut state, id, STATUS_CONFLICT));
    }
    let commit = RelayReallocationCommit {
        protocol_version: PROTOCOL_VERSION,
        reallocation_id: Bytes::copy_from_slice(&id),
        request_id: Bytes::copy_from_slice(&txn.request_id),
        session_uuid: txn.binding.session_uuid.clone(),
        relay_server: txn.new_relay.clone(),
        node_id: txn.new_node_id.clone(),
        old_session_generation: txn.binding.session_generation,
        new_session_generation: txn.new_generation,
        config_generation: txn.binding.config_generation,
        path_binding_sha256: Bytes::copy_from_slice(&controller.binding),
        controller_authorization: txn.controller_authorization.clone(),
        target_authorization: txn.target_authorization.clone(),
        reason_code: 7,
        drain_after_unix_ms: unix_ms().saturating_add(2_000),
        ..Default::default()
    };
    txn.phase = Phase::Commit;
    txn.commit = Some(commit.clone());
    let dispatch = dispatch_for(&txn.binding, ResultPayload::Commit(commit));
    let _ = txn;
    if accepted {
        state.counters.ready_accepted += 1;
    } else {
        state.counters.ready_rejected += 1;
    }
    state.counters.committed += 1;
    Some(dispatch)
}

pub(crate) fn install_authorizations(
    reallocation_id: &[u8],
    controller: Bytes,
    target: Bytes,
) -> bool {
    let Ok(id) = <[u8; ID_BYTES]>::try_from(reallocation_id) else {
        return false;
    };
    let Ok(mut state) = STATE.write() else {
        return false;
    };
    let Some(txn) = state.active.get_mut(&id) else {
        return false;
    };
    if txn.phase != Phase::Prepare {
        return false;
    }
    txn.controller_authorization = controller;
    txn.target_authorization = target;
    true
}

pub(crate) fn commit_ack(
    message: RelayReallocationCommitAck,
    route: SocketAddr,
    ip: IpAddr,
) -> Option<Dispatch> {
    let id: [u8; ID_BYTES] = message.reallocation_id.as_ref().try_into().ok()?;
    let mut state = STATE.write().ok()?;
    let txn = state.active.get_mut(&id)?;
    let commit = txn.commit.as_ref()?;
    if txn.phase != Phase::Commit
        || !message.installed
        || message.request_id != commit.request_id
        || message.session_uuid != commit.session_uuid
        || message.new_session_generation != commit.new_session_generation
        || message.config_generation != commit.config_generation
        || message.path_binding_sha256 != commit.path_binding_sha256
    {
        return Some(rollback_active(&mut state, id, STATUS_CONNECT_FAILED));
    }
    match message.endpoint_role {
        ROLE_CONTROLLER
            if normalize_route(route) == normalize_route(txn.binding.controller_route)
                && normalize_ip(ip) == normalize_ip(txn.binding.controller_ip) =>
        {
            txn.controller_committed = true
        }
        ROLE_TARGET
            if normalize_route(route) == normalize_route(txn.binding.target_route)
                && normalize_ip(ip) == normalize_ip(txn.binding.target_ip) =>
        {
            txn.target_committed = true
        }
        _ => return None,
    };
    if !(txn.controller_committed && txn.target_committed) {
        return None;
    }
    let result = dispatch_for(&txn.binding, ResultPayload::Commit(commit.clone()));
    let mut binding = txn.binding.clone();
    binding.relay_server = txn.new_relay.clone();
    binding.session_generation = txn.new_generation;
    let session_uuid = binding.session_uuid.clone();
    let _ = txn;
    state.sessions.insert(session_uuid.clone(), binding);
    state.by_session.remove(&session_uuid);
    state.active.remove(&id);
    state.counters.commit_acked += 1;
    Some(result)
}

pub(crate) fn expire() -> Vec<Dispatch> {
    let now = Instant::now();
    let Ok(mut state) = STATE.write() else {
        return Vec::new();
    };
    let ids = state
        .active
        .iter()
        .filter_map(|(id, t)| (t.deadline <= now).then_some(*id))
        .collect::<Vec<_>>();
    ids.into_iter()
        .map(|id| {
            state.counters.expired += 1;
            rollback_active(&mut state, id, STATUS_PEER_TIMEOUT)
        })
        .collect()
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    let active = starry_config::active_snapshot();
    let config = active.config.as_ref().map(|c| &c.relay_reallocation);
    let state = STATE.read().ok();
    let c = state
        .as_ref()
        .map(|s| &s.counters)
        .cloned()
        .unwrap_or_default();
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        enabled: config.is_some_and(|v| v.enabled),
        policy: config
            .map(|v| {
                match v.policy {
                    RelayReallocationPolicyConfig::Auto => "auto",
                    RelayReallocationPolicyConfig::Fixed => "fixed",
                    RelayReallocationPolicyConfig::ForceAuto => "force-auto",
                    RelayReallocationPolicyConfig::ForceFixed => "force-fixed",
                }
                .to_owned()
            })
            .unwrap_or_else(|| "auto".to_owned()),
        active: state.as_ref().map(|s| s.active.len()).unwrap_or_default(),
        configured_nodes: active
            .config
            .as_ref()
            .map(|v| {
                v.websocket_signal
                    .relay_health
                    .endpoints
                    .iter()
                    .filter(|e| e.node_id.is_some())
                    .count()
            })
            .unwrap_or_default(),
        requested: c.requested,
        accepted: c.accepted,
        idempotent: c.idempotent,
        simultaneous: c.simultaneous,
        conflicts: c.conflicts,
        rate_limited: c.rate_limited,
        binding_mismatch: c.binding_mismatch,
        prepared: c.prepared,
        ready_accepted: c.ready_accepted,
        ready_rejected: c.ready_rejected,
        committed: c.committed,
        commit_acked: c.commit_acked,
        rolled_back: c.rolled_back,
        expired: c.expired,
        generation_changed: c.generation_changed,
        peer_timeout: c.peer_timeout,
        connect_failed: c.connect_failed,
        fastmedia_fenced: c.fastmedia_fenced,
    }
}

fn choose_relay(
    config: &starry_config::StarryConfig,
    request: &RelayReallocationRequest,
    old: &str,
) -> Option<(String, String)> {
    let fixed = matches!(
        config.relay_reallocation.policy,
        RelayReallocationPolicyConfig::Fixed | RelayReallocationPolicyConfig::ForceFixed
    );
    let wanted = if fixed {
        Some(config.relay_reallocation.fixed_node_id.as_str())
    } else if request.preferred_node_id.is_empty() {
        None
    } else {
        Some(request.preferred_node_id.as_str())
    };
    let health = crate::websocket_signal::health_runtime_snapshot(
        config.relay_quality.max_telemetry_age_seconds,
    );
    config
        .websocket_signal
        .relay_health
        .endpoints
        .iter()
        .filter(|e| !e.relay.eq_ignore_ascii_case(old))
        .filter(|e| {
            health.endpoint(&e.relay).is_some_and(|r| {
                r.state == "healthy" && !r.stale && r.relay_probe_protocol == Some(1)
            })
        })
        .find(|e| wanted.is_none_or(|w| e.node_id.as_deref() == Some(w)))
        .and_then(|e| Some((e.node_id.clone()?, e.relay.clone())))
}

fn prepare_for(t: &Transaction, role: u32) -> RelayReallocationPrepare {
    RelayReallocationPrepare {
        protocol_version: PROTOCOL_VERSION,
        reallocation_id: Bytes::copy_from_slice(&t.reallocation_id),
        request_id: Bytes::copy_from_slice(&t.request_id),
        session_uuid: t.binding.session_uuid.clone(),
        endpoint_role: role,
        old_relay_server: t.binding.relay_server.clone(),
        new_relay_server: t.new_relay.clone(),
        new_node_id: t.new_node_id.clone(),
        old_session_generation: t.binding.session_generation,
        new_session_generation: t.new_generation,
        config_generation: t.binding.config_generation,
        prepare_token: Bytes::copy_from_slice(&t.prepare_token),
        deadline_unix_ms: t.deadline_unix_ms,
        ..Default::default()
    }
}
fn dispatch_for(b: &SessionBinding, payload: ResultPayload) -> Dispatch {
    Dispatch {
        controller_route: b.controller_route,
        target_route: b.target_route,
        controller_id: b.controller_id.clone(),
        target_id: b.target_id.clone(),
        controller_websocket: b.controller_websocket,
        target_websocket: b.target_websocket,
        controller_ip: b.controller_ip,
        target_ip: b.target_ip,
        payload,
        snapshot: None,
    }
}
fn rollback_active(state: &mut State, id: [u8; ID_BYTES], status: u32) -> Dispatch {
    let t = state.active.remove(&id).expect("active");
    state.by_session.remove(&t.binding.session_uuid);
    state.counters.rolled_back += 1;
    if status == STATUS_PEER_TIMEOUT {
        state.counters.peer_timeout += 1;
    }
    if status == STATUS_CONNECT_FAILED {
        state.counters.connect_failed += 1;
    }
    let r = RelayReallocationRollback {
        protocol_version: PROTOCOL_VERSION,
        reallocation_id: Bytes::copy_from_slice(&id),
        request_id: Bytes::copy_from_slice(&t.request_id),
        status,
        relay_server: t.binding.relay_server.clone(),
        session_generation: t.binding.session_generation,
        config_generation: t.binding.config_generation,
        reconnect_both: false,
        ..Default::default()
    };
    dispatch_for(&t.binding, ResultPayload::Rollback(r))
}
fn rollback_for(
    r: &RelayReallocationRequest,
    status: u32,
    relay: &str,
    generation: u64,
    config_generation: u64,
) -> RelayReallocationRollback {
    RelayReallocationRollback {
        protocol_version: PROTOCOL_VERSION,
        request_id: r.request_id.clone(),
        status,
        relay_server: relay.to_owned(),
        session_generation: generation,
        config_generation,
        ..Default::default()
    }
}
fn snapshot_error(id: &[u8], status: u32) -> RelayReallocationCandidateSnapshot {
    RelayReallocationCandidateSnapshot {
        protocol_version: PROTOCOL_VERSION,
        request_id: Bytes::copy_from_slice(id),
        status,
        ..Default::default()
    }
}
fn request_digest(r: &RelayReallocationRequest) -> Option<[u8; 32]> {
    use hbb_common::protobuf::Message as _;
    let bytes = r.write_to_bytes().ok()?;
    Some(Sha256::digest(bytes).into())
}
fn conflict_key(r: &RelayReallocationRequest) -> (u64, u8, [u8; 16]) {
    let mut id = [0; 16];
    if r.request_id.len() == 16 {
        id.copy_from_slice(&r.request_id);
    }
    (r.deadline_unix_ms, role_priority(r.endpoint_role), id)
}
fn role_priority(role: u32) -> u8 {
    if role == ROLE_CONTROLLER {
        0
    } else {
        1
    }
}
fn consume_rate(
    state: &mut State,
    session: &str,
    ip: IpAddr,
    p: &RelayReallocationConfig,
    now: Instant,
) -> bool {
    while state
        .global
        .front()
        .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
    {
        state.global.pop_front();
    }
    let q = state.per_ip.entry(ip).or_default();
    while q
        .front()
        .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
    {
        q.pop_front();
    }
    let sq = state.per_session.entry(session.to_owned()).or_default();
    while sq
        .front()
        .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
    {
        sq.pop_front();
    }
    if state.global.len() >= p.global_per_minute as usize
        || q.len() >= p.per_ip_per_minute as usize
        || sq.len() >= p.per_session_per_minute as usize
    {
        return false;
    }
    state.global.push_back(now);
    q.push_back(now);
    sq.push_back(now);
    true
}
fn cleanup(state: &mut State, p: &RelayReallocationConfig, now: Instant) {
    state
        .completed
        .retain(|_, v| now.duration_since(v.at) < RESULT_REPLAY_TTL);
    while state
        .global
        .front()
        .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
    {
        state.global.pop_front();
    }
    state.per_ip.retain(|_, q| {
        while q
            .front()
            .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
        {
            q.pop_front();
        }
        !q.is_empty()
    });
    state.per_session.retain(|_, q| {
        while q
            .front()
            .is_some_and(|v| now.duration_since(*v) >= RATE_WINDOW)
        {
            q.pop_front();
        }
        !q.is_empty()
    });
    if state.active.len() > p.max_active {
        let mut ids = state
            .active
            .iter()
            .map(|(id, t)| (*id, t.deadline))
            .collect::<Vec<_>>();
        ids.sort_by_key(|v| v.1);
        for (id, _) in ids.into_iter().take(state.active.len() - p.max_active) {
            state.active.remove(&id);
        }
    }
}
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v)),
        v => v,
    }
}
fn normalize_route(mut a: SocketAddr) -> SocketAddr {
    a.set_ip(normalize_ip(a.ip()));
    a
}
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
fn fresh_id() -> [u8; 16] {
    *uuid::Uuid::now_v7().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_priority_is_deadline_then_controller_then_id() {
        let mut a = RelayReallocationRequest {
            deadline_unix_ms: 2,
            endpoint_role: ROLE_TARGET,
            request_id: Bytes::from_static(&[2; 16]),
            ..Default::default()
        };
        let mut b = a.clone();
        b.deadline_unix_ms = 1;
        assert!(conflict_key(&b) < conflict_key(&a));
        b.deadline_unix_ms = 2;
        b.endpoint_role = ROLE_CONTROLLER;
        assert!(conflict_key(&b) < conflict_key(&a));
        a.endpoint_role = ROLE_CONTROLLER;
        a.request_id = Bytes::from_static(&[3; 16]);
        assert!(conflict_key(&b) < conflict_key(&a));
    }
    #[test]
    fn bounded_rate_uses_controlled_clock() {
        let mut s = State::default();
        let p = RelayReallocationConfig {
            per_session_per_minute: 1,
            per_ip_per_minute: 1,
            global_per_minute: 2,
            ..Default::default()
        };
        let now = Instant::now();
        let ip = "192.0.2.1".parse().unwrap();
        assert!(consume_rate(&mut s, "session", ip, &p, now));
        assert!(!consume_rate(
            &mut s,
            "session",
            ip,
            &p,
            now + Duration::from_secs(59)
        ));
        assert!(consume_rate(
            &mut s,
            "session",
            ip,
            &p,
            now + Duration::from_secs(61)
        ));
    }
    #[test]
    fn exact_alias_binding_does_not_infer_hosts() {
        let endpoint = crate::starry_config::RelayEndpointConfig {
            relay: "relay.example:21117".into(),
            url: "wss://probe.example/ws/relay".into(),
            node_id: Some("n1".into()),
            display_name: Some("Relay One".into()),
            region: Some("test".into()),
            relay_server_aliases: vec!["alias.example:21117".into()],
            probe_url_aliases: vec![],
            ..Default::default()
        };
        assert!(!endpoint
            .relay_server_aliases
            .iter()
            .any(|v| v == "evil.example:21117"));
    }
}
