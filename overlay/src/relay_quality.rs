use crate::{
    relay_observer::{self, RelayRuntimeView},
    starry_config::{self, RelayQualityConfig, RelayQualityStrategyConfig},
    websocket_signal::RelayRequirement,
};
use hbb_common::rendezvous_proto::{
    RelayProbeReport, RelayProbeResult, RelayQualityCancel, RelayQualityCandidate,
    RelayQualityDecision, RelayQualityOffer, RelayQualityScore,
};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const STRATEGY_ADAPTIVE: u32 = 1;
pub(crate) const STRATEGY_EAGER: u32 = 2;
pub(crate) const STAGE_PRIMARY: u32 = 1;
pub(crate) const STAGE_EXPANDED: u32 = 2;
pub(crate) const STAGE_EAGER: u32 = 3;
pub(crate) const ENDPOINT_CONTROLLER: u32 = 1;
pub(crate) const ENDPOINT_TARGET: u32 = 2;
pub(crate) const DECISION_PRIMARY_ACCEPTED: u32 = 1;
pub(crate) const DECISION_EXPANDED_BEST_SCORE: u32 = 2;
pub(crate) const DECISION_PARTIAL: u32 = 3;
pub(crate) const DECISION_HYSTERESIS: u32 = 4;
pub(crate) const DECISION_LEGACY_FALLBACK: u32 = 5;
pub(crate) const DECISION_PROBE_FAILURE: u32 = 6;
pub(crate) const DECISION_MANUAL_OVERRIDE: u32 = 7;
pub(crate) const CANCEL_P2P_SUCCEEDED: u32 = 1;
pub(crate) const CANCEL_CLIENT_ABORT: u32 = 2;

const SCORE_MAX: u32 = 10_000;
const UNKNOWN_LOAD_PENALTY: u32 = 5_000;
const MAX_REPORTED_RTT_MS: u32 = 120_000;
const MAX_RELAY_COUNTER_DIMENSION: usize = 256;
const PRIMARY_SIGNAL_MARGIN_MS: u64 = 500;
pub(crate) const MAX_DEADLINE_DISPATCHES_PER_TICK: usize = 64;

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct OfferSkipCounters {
    pub(crate) disabled: u64,
    pub(crate) unsupported_client: u64,
    pub(crate) invalid_fallback: u64,
    pub(crate) inconsistent_snapshot: u64,
    pub(crate) insufficient_candidates: u64,
    pub(crate) primary_not_probeable: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct FallbackReasonCounters {
    pub(crate) legacy_fallback: u64,
    pub(crate) probe_failure: u64,
    pub(crate) manual_override: u64,
    pub(crate) invalid_report: u64,
    pub(crate) report_late: u64,
}

#[derive(Clone, Copy)]
enum OfferSkipReason {
    Disabled,
    UnsupportedClient,
    InvalidFallback,
    InconsistentSnapshot,
    InsufficientCandidates,
    PrimaryNotProbeable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Primary,
    Expanded,
    Eager,
}

impl Stage {
    fn wire(self) -> u32 {
        match self {
            Self::Primary => STAGE_PRIMARY,
            Self::Expanded => STAGE_EXPANDED,
            Self::Eager => STAGE_EAGER,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndpointRole {
    Controller,
    Target,
}

impl EndpointRole {
    fn wire(self) -> u32 {
        match self {
            Self::Controller => ENDPOINT_CONTROLLER,
            Self::Target => ENDPOINT_TARGET,
        }
    }

    fn from_wire(value: u32) -> Option<Self> {
        match value {
            ENDPOINT_CONTROLLER => Some(Self::Controller),
            ENDPOINT_TARGET => Some(Self::Target),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum DecisionReason {
    PrimaryAccepted,
    ExpandedBestScore,
    Partial,
    Hysteresis,
    LegacyFallback,
    ProbeFailure,
    ManualOverride,
}

impl DecisionReason {
    fn code(self) -> u32 {
        match self {
            Self::PrimaryAccepted => DECISION_PRIMARY_ACCEPTED,
            Self::ExpandedBestScore => DECISION_EXPANDED_BEST_SCORE,
            Self::Partial => DECISION_PARTIAL,
            Self::Hysteresis => DECISION_HYSTERESIS,
            Self::LegacyFallback => DECISION_LEGACY_FALLBACK,
            Self::ProbeFailure => DECISION_PROBE_FAILURE,
            Self::ManualOverride => DECISION_MANUAL_OVERRIDE,
        }
    }
}

static STATE: Lazy<RwLock<QualityState>> = Lazy::new(|| RwLock::new(QualityState::default()));

#[derive(Default)]
struct QualityState {
    allocations: HashMap<Vec<u8>, Allocation>,
    routes: HashMap<RouteKey, Vec<u8>>,
    finalized_allocations: HashMap<Vec<u8>, FinalizedAllocation>,
    request_decisions: HashMap<RequestDecisionKey, DecisionRecord>,
    decisions: HashMap<DecisionKey, DecisionRecord>,
    cache: HashMap<CacheKey, CachedSelection>,
    offers_created: u64,
    offers_skipped: u64,
    offer_skip_reasons: OfferSkipCounters,
    peer_reports_accepted: u64,
    controller_reports_accepted: u64,
    reports_accepted: u64,
    reports_duplicate: u64,
    reports_stage_mismatch: u64,
    reports_late: u64,
    reports_invalid: u64,
    reports_binding_mismatch: u64,
    decisions_created: u64,
    fallback_decisions: u64,
    fallback_reasons: FallbackReasonCounters,
    cache_hits: u64,
    hysteresis_decisions: u64,
    primary_probes: u64,
    primary_accepted: u64,
    expansions_triggered: u64,
    p2p_cancellations: u64,
    estimated_probe_attempts_saved: u64,
    expanded_decisions: u64,
    stage_timeouts: u64,
    relay_selections: HashMap<String, u64>,
    relay_selection_overflow: u64,
}

struct Allocation {
    created: Instant,
    total_deadline: Instant,
    total_deadline_unix_ms: u64,
    stage_deadline: Instant,
    stage_deadline_unix_ms: u64,
    config_generation: u64,
    initiator_route: SocketAddr,
    initiator_ip: IpAddr,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_id: String,
    target_websocket: bool,
    requirement: RelayRequirement,
    fallback_relay: String,
    primary_relay: String,
    allocation_id: Vec<u8>,
    stage_token: Vec<u8>,
    stage: Stage,
    candidates: Vec<CandidateState>,
    config: RelayQualityConfig,
    reports: StageReports,
    pending_uuid: Option<String>,
}

#[derive(Clone)]
struct CandidateState {
    relay: String,
    server_rtt_ms: Option<u32>,
    load_basis_points: Option<u32>,
    controller_wire: RelayQualityCandidate,
    target_wire: RelayQualityCandidate,
}

#[derive(Clone, Default)]
struct StageReports {
    primary_controller: Option<ValidatedReport>,
    primary_target: Option<ValidatedReport>,
    expanded_controller: Option<ValidatedReport>,
    expanded_target: Option<ValidatedReport>,
    eager_controller: Option<ValidatedReport>,
    eager_target: Option<ValidatedReport>,
}

impl StageReports {
    fn get(&self, stage: Stage, role: EndpointRole) -> Option<&ValidatedReport> {
        match (stage, role) {
            (Stage::Primary, EndpointRole::Controller) => self.primary_controller.as_ref(),
            (Stage::Primary, EndpointRole::Target) => self.primary_target.as_ref(),
            (Stage::Expanded, EndpointRole::Controller) => self.expanded_controller.as_ref(),
            (Stage::Expanded, EndpointRole::Target) => self.expanded_target.as_ref(),
            (Stage::Eager, EndpointRole::Controller) => self.eager_controller.as_ref(),
            (Stage::Eager, EndpointRole::Target) => self.eager_target.as_ref(),
        }
    }

    fn slot_mut(&mut self, stage: Stage, role: EndpointRole) -> &mut Option<ValidatedReport> {
        match (stage, role) {
            (Stage::Primary, EndpointRole::Controller) => &mut self.primary_controller,
            (Stage::Primary, EndpointRole::Target) => &mut self.primary_target,
            (Stage::Expanded, EndpointRole::Controller) => &mut self.expanded_controller,
            (Stage::Expanded, EndpointRole::Target) => &mut self.expanded_target,
            (Stage::Eager, EndpointRole::Controller) => &mut self.eager_controller,
            (Stage::Eager, EndpointRole::Target) => &mut self.eager_target,
        }
    }
}

#[derive(Clone, Default)]
struct ValidatedReport {
    metrics: HashMap<String, Metric>,
    wire: RelayProbeReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReportRejection {
    Late,
    Invalid,
    StageMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreReportResult {
    Accepted,
    Duplicate,
    Conflict,
}

#[derive(Clone, Copy)]
struct Metric {
    attempted: u32,
    succeeded: u32,
    rtt_ms: u32,
    jitter_ms: u32,
}

#[derive(Clone, Copy, Eq)]
struct RouteKey {
    initiator: SocketAddr,
    target_ip: IpAddr,
}

impl PartialEq for RouteKey {
    fn eq(&self, other: &Self) -> bool {
        self.initiator == other.initiator && self.target_ip == other.target_ip
    }
}

impl Hash for RouteKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.initiator.hash(state);
        self.target_ip.hash(state);
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct CacheKey {
    endpoint_a: NetworkPrefix,
    endpoint_b: NetworkPrefix,
    requirement: u8,
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum NetworkPrefix {
    V4([u8; 3]),
    V6([u8; 7]),
}

struct CachedSelection {
    relay: String,
    created: Instant,
}

#[derive(Clone)]
struct DecisionRecord {
    decision: RelayQualityDecision,
    allocation_id: Vec<u8>,
    created: Instant,
    config_generation: u64,
    target_ip: IpAddr,
}

#[derive(Clone)]
struct FinalizedAllocation {
    decision: RelayQualityDecision,
    created: Instant,
    config_generation: u64,
    initiator_route: SocketAddr,
    initiator_ip: IpAddr,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_id: String,
    target_websocket: bool,
    stage: Stage,
    stage_token: Vec<u8>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct RequestDecisionKey {
    uuid: String,
    initiator_ip: IpAddr,
    target_ip: IpAddr,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct DecisionKey {
    uuid: String,
    target_ip: IpAddr,
}

pub(crate) struct ResponseContext {
    pub(crate) offer: Option<RelayQualityOffer>,
    pub(crate) peer_report: Option<RelayProbeReport>,
}

#[derive(Clone)]
pub(crate) struct RelaySelection {
    pub(crate) decision: RelayQualityDecision,
    pub(crate) target_ip: IpAddr,
    pub(crate) config_generation: u64,
}

#[derive(Clone)]
pub(crate) enum DispatchPayload {
    Offer(RelayQualityOffer),
    Decision(RelayQualityDecision),
    Cancel(RelayQualityCancel),
}

#[derive(Clone)]
pub(crate) struct StageDispatch {
    pub(crate) controller_route: SocketAddr,
    pub(crate) target_route: SocketAddr,
    pub(crate) target_id: String,
    pub(crate) target_websocket: bool,
    pub(crate) controller: DispatchPayload,
    pub(crate) target: DispatchPayload,
}

pub(crate) enum RequestResolution {
    Legacy,
    Selected(RelaySelection),
    Expanded(StageDispatch),
    Pending,
    Blocked,
}

pub(crate) enum StageReportResolution {
    Pending,
    Dispatch(StageDispatch),
    Rejected,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) strategy: String,
    pub(crate) enabled: bool,
    pub(crate) active_allocations: usize,
    pub(crate) cached_network_pairs: usize,
    pub(crate) pending_decisions: usize,
    pub(crate) offers_created: u64,
    pub(crate) offers_skipped: u64,
    pub(crate) offer_skip_reasons: OfferSkipCounters,
    pub(crate) peer_reports_accepted: u64,
    pub(crate) controller_reports_accepted: u64,
    pub(crate) reports_accepted: u64,
    pub(crate) reports_duplicate: u64,
    pub(crate) reports_stage_mismatch: u64,
    pub(crate) reports_late: u64,
    pub(crate) reports_invalid: u64,
    pub(crate) reports_binding_mismatch: u64,
    pub(crate) decisions_created: u64,
    pub(crate) fallback_decisions: u64,
    pub(crate) fallback_reasons: FallbackReasonCounters,
    pub(crate) cache_hits: u64,
    pub(crate) hysteresis_decisions: u64,
    pub(crate) primary_probes: u64,
    pub(crate) primary_accepted: u64,
    pub(crate) expansions_triggered: u64,
    pub(crate) p2p_cancellations: u64,
    pub(crate) estimated_probe_attempts_saved: u64,
    pub(crate) expanded_decisions: u64,
    pub(crate) stage_timeouts: u64,
    pub(crate) relay_selections: HashMap<String, u64>,
    pub(crate) relay_selection_overflow: u64,
}

pub(crate) fn create_offer(
    client_protocol: u32,
    initiator_route: SocketAddr,
    initiator_ip: IpAddr,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_id: &str,
    requirement: RelayRequirement,
    initiator_websocket: bool,
    target_websocket: bool,
    fallback: &str,
) -> Option<RelayQualityOffer> {
    let active = starry_config::active_snapshot();
    let Some(config) = active
        .config
        .as_ref()
        .map(|config| config.relay_quality.clone())
    else {
        record_offer_skipped(OfferSkipReason::Disabled);
        return None;
    };
    if !config.enabled {
        record_offer_skipped(OfferSkipReason::Disabled);
        return None;
    }
    if client_protocol != PROTOCOL_VERSION {
        record_offer_skipped(OfferSkipReason::UnsupportedClient);
        return None;
    }
    if fallback.is_empty() || target_id.is_empty() || config.max_candidates < 2 {
        record_offer_skipped(OfferSkipReason::InvalidFallback);
        return None;
    }
    let snapshot = relay_observer::snapshot();
    if !snapshot.is_consistent() || snapshot.config_generation != active.generation {
        record_offer_skipped(OfferSkipReason::InconsistentSnapshot);
        return None;
    }
    let views = snapshot.quality_candidates(
        initiator_ip,
        target_ip,
        requirement,
        fallback,
        config.max_candidates,
    );
    if views.len() < 2 {
        record_offer_skipped(OfferSkipReason::InsufficientCandidates);
        return None;
    }
    if !views[0].id.eq_ignore_ascii_case(fallback) {
        // Adaptive probing is GEO-primary-first. A legacy or telemetry-stale
        // primary remains an ordinary fallback and suppresses the quality
        // allocation instead of silently probing a different Relay first.
        record_offer_skipped(OfferSkipReason::PrimaryNotProbeable);
        return None;
    }

    let allocation_id = uuid::Uuid::now_v7().as_bytes().to_vec();
    let stage_token = fresh_stage_token();
    let candidates = views
        .iter()
        .map(|view| CandidateState {
            relay: view.id.clone(),
            server_rtt_ms: view
                .websocket
                .latency_ms
                .map(|value| value.min(u64::from(u32::MAX)) as u32),
            load_basis_points: view.websocket.load_basis_points,
            controller_wire: wire_candidate(view, initiator_websocket),
            target_wire: wire_candidate(view, target_websocket),
        })
        .collect::<Vec<_>>();
    let created = Instant::now();
    let now_unix_ms = unix_millis();
    let total_ms = u64::from(config.report_timeout_ms);
    let (stage, stage_budget_ms) = match config.strategy {
        RelayQualityStrategyConfig::Adaptive => (
            Stage::Primary,
            primary_stage_budget_ms(&config).min(total_ms),
        ),
        RelayQualityStrategyConfig::Eager => (Stage::Eager, total_ms),
    };
    let allocation = Allocation {
        created,
        total_deadline: checked_deadline(created, total_ms),
        total_deadline_unix_ms: now_unix_ms.saturating_add(total_ms),
        stage_deadline: checked_deadline(created, stage_budget_ms),
        stage_deadline_unix_ms: now_unix_ms.saturating_add(stage_budget_ms),
        config_generation: active.generation,
        initiator_route: normalize_socket(initiator_route),
        initiator_ip: normalize_ip(initiator_ip),
        target_route: normalize_socket(target_route),
        target_ip: normalize_ip(target_ip),
        target_id: target_id.to_owned(),
        target_websocket,
        requirement,
        fallback_relay: fallback.to_owned(),
        primary_relay: views[0].id.clone(),
        allocation_id: allocation_id.clone(),
        stage_token,
        stage,
        candidates,
        config: config.clone(),
        reports: StageReports::default(),
        pending_uuid: None,
    };
    let target_offer = offer_for(&allocation, EndpointRole::Target);
    let route = RouteKey {
        initiator: allocation.initiator_route,
        target_ip: allocation.target_ip,
    };
    let mut state = STATE.write().ok()?;
    cleanup(&mut state, &config);
    if let Some(previous) = state.routes.insert(route, allocation_id.clone()) {
        state.allocations.remove(&previous);
    }
    ensure_allocation_capacity(&mut state, config.max_allocations);
    state.allocations.insert(allocation_id, allocation);
    state.offers_created = state.offers_created.saturating_add(1);
    Some(target_offer)
}

fn record_offer_skipped(reason: OfferSkipReason) {
    let Ok(mut state) = STATE.write() else {
        return;
    };
    state.offers_skipped = state.offers_skipped.saturating_add(1);
    let counter = match reason {
        OfferSkipReason::Disabled => &mut state.offer_skip_reasons.disabled,
        OfferSkipReason::UnsupportedClient => &mut state.offer_skip_reasons.unsupported_client,
        OfferSkipReason::InvalidFallback => &mut state.offer_skip_reasons.invalid_fallback,
        OfferSkipReason::InconsistentSnapshot => {
            &mut state.offer_skip_reasons.inconsistent_snapshot
        }
        OfferSkipReason::InsufficientCandidates => {
            &mut state.offer_skip_reasons.insufficient_candidates
        }
        OfferSkipReason::PrimaryNotProbeable => &mut state.offer_skip_reasons.primary_not_probeable,
    };
    *counter = counter.saturating_add(1);
}

pub(crate) fn response_context(
    initiator_route: SocketAddr,
    target_route: SocketAddr,
    target_ip: IpAddr,
    target_id: &str,
    report: Option<RelayProbeReport>,
) -> ResponseContext {
    let active = starry_config::active_snapshot();
    let Some(config) = active
        .config
        .as_ref()
        .map(|config| config.relay_quality.clone())
    else {
        return empty_response_context();
    };
    if !config.enabled {
        return empty_response_context();
    }
    let key = RouteKey {
        initiator: normalize_socket(initiator_route),
        target_ip: normalize_ip(target_ip),
    };
    let Ok(mut state) = STATE.write() else {
        return empty_response_context();
    };
    cleanup(&mut state, &config);
    let Some(allocation_id) = state.routes.get(&key).cloned() else {
        if report.is_some() {
            state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        }
        return empty_response_context();
    };
    let mut report_result = None;
    let (offer, peer_report) = {
        let Some(allocation) = state.allocations.get_mut(&allocation_id) else {
            if report.is_some() {
                state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
            }
            return empty_response_context();
        };
        if allocation.config_generation != active.generation
            || !initial_response_source_matches(
                allocation,
                normalize_socket(target_route),
                normalize_ip(target_ip),
                target_id,
            )
        {
            if report.is_some() {
                report_result = Some(ReportAccounting::BindingMismatch);
            }
            (None, None)
        } else {
            if let Some(report) = report {
                report_result = Some(
                    match validate_report_until(
                        &report,
                        allocation,
                        EndpointRole::Target,
                        Instant::now(),
                    ) {
                        Ok(validated) => match store_report(
                            allocation
                                .reports
                                .slot_mut(allocation.stage, EndpointRole::Target),
                            validated,
                        ) {
                            StoreReportResult::Accepted => ReportAccounting::Accepted {
                                role: EndpointRole::Target,
                                stage: allocation.stage,
                            },
                            StoreReportResult::Duplicate => ReportAccounting::Duplicate,
                            StoreReportResult::Conflict => ReportAccounting::Invalid,
                        },
                        Err(ReportRejection::Late) => ReportAccounting::Late,
                        Err(ReportRejection::StageMismatch) => ReportAccounting::StageMismatch,
                        Err(ReportRejection::Invalid) => ReportAccounting::Invalid,
                    },
                );
            }
            let offer = (Instant::now() <= allocation.stage_deadline)
                .then(|| offer_for(allocation, EndpointRole::Controller));
            let peer_report = allocation
                .reports
                .get(allocation.stage, EndpointRole::Target)
                .map(|report| report.wire.clone());
            (offer, peer_report)
        }
    };
    if let Some(result) = report_result {
        account_report(&mut state, result);
    }
    ResponseContext { offer, peer_report }
}

pub(crate) fn select_for_request(
    source_route: SocketAddr,
    source_ip: IpAddr,
    target_ip: IpAddr,
    uuid: &str,
    explicit_allocation_id: &[u8],
    report: Option<RelayProbeReport>,
) -> RequestResolution {
    let active = starry_config::active_snapshot();
    let Some(config) = active
        .config
        .as_ref()
        .map(|value| value.relay_quality.clone())
    else {
        return RequestResolution::Legacy;
    };
    if !config.enabled {
        return RequestResolution::Legacy;
    }
    let source_route = normalize_socket(source_route);
    let source_ip = normalize_ip(source_ip);
    let target_ip = normalize_ip(target_ip);
    let mut state = match STATE.write() {
        Ok(state) => state,
        Err(_) => return RequestResolution::Blocked,
    };
    cleanup(&mut state, &config);
    let request_key = RequestDecisionKey {
        uuid: uuid.to_owned(),
        initiator_ip: source_ip,
        target_ip,
    };
    if !uuid.is_empty() {
        if let Some(record) = state.request_decisions.get(&request_key) {
            let report_allocation = report.as_ref().map(|value| value.allocation_id.as_ref());
            let binding = if !explicit_allocation_id.is_empty() {
                explicit_allocation_id
            } else {
                report_allocation.unwrap_or_default()
            };
            if record.config_generation == active.generation
                && binding == record.allocation_id.as_slice()
            {
                return RequestResolution::Selected(RelaySelection {
                    decision: record.decision.clone(),
                    target_ip: record.target_ip,
                    config_generation: record.config_generation,
                });
            }
            state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
            return RequestResolution::Blocked;
        }
    }
    if report.is_none() && explicit_allocation_id.is_empty() {
        return RequestResolution::Legacy;
    }
    let Some(report) = report else {
        state.reports_invalid = state.reports_invalid.saturating_add(1);
        return RequestResolution::Blocked;
    };
    let allocation_id = report.allocation_id.to_vec();
    if (!explicit_allocation_id.is_empty() && explicit_allocation_id != allocation_id.as_slice())
        || allocation_id.len() != 16
    {
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return RequestResolution::Blocked;
    }
    let Some(mut allocation) = state.allocations.remove(&allocation_id) else {
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return RequestResolution::Blocked;
    };
    if allocation.config_generation != active.generation
        || !source_matches(
            &allocation,
            source_route,
            source_ip,
            EndpointRole::Controller,
        )
        || allocation.target_ip != target_ip
        || uuid.is_empty()
        || allocation
            .pending_uuid
            .as_ref()
            .map(|pending| pending != uuid)
            .unwrap_or(false)
    {
        state.allocations.insert(allocation_id, allocation);
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return RequestResolution::Blocked;
    }
    allocation.pending_uuid = Some(uuid.to_owned());
    let validated = match validate_report_until(
        &report,
        &allocation,
        EndpointRole::Controller,
        Instant::now(),
    ) {
        Ok(validated) => validated,
        Err(ReportRejection::Late) => {
            state.reports_late = state.reports_late.saturating_add(1);
            state.stage_timeouts = state.stage_timeouts.saturating_add(1);
            let decision =
                fallback_decision(&allocation, allocation.stage, DecisionReason::ProbeFailure);
            let selection = record_final_decision(
                &mut state,
                allocation,
                decision,
                None,
                Some(FallbackAccounting::ReportLate),
            );
            return RequestResolution::Selected(selection);
        }
        Err(ReportRejection::StageMismatch) => {
            state.reports_stage_mismatch = state.reports_stage_mismatch.saturating_add(1);
            state.allocations.insert(allocation_id, allocation);
            return RequestResolution::Blocked;
        }
        Err(ReportRejection::Invalid) => {
            state.reports_invalid = state.reports_invalid.saturating_add(1);
            let decision =
                fallback_decision(&allocation, allocation.stage, DecisionReason::ProbeFailure);
            let selection = record_final_decision(
                &mut state,
                allocation,
                decision,
                None,
                Some(FallbackAccounting::InvalidReport),
            );
            return RequestResolution::Selected(selection);
        }
    };
    match store_report(
        allocation
            .reports
            .slot_mut(allocation.stage, EndpointRole::Controller),
        validated,
    ) {
        StoreReportResult::Accepted => account_report(
            &mut state,
            ReportAccounting::Accepted {
                role: EndpointRole::Controller,
                stage: allocation.stage,
            },
        ),
        StoreReportResult::Duplicate => {
            state.reports_duplicate = state.reports_duplicate.saturating_add(1)
        }
        StoreReportResult::Conflict => {
            state.reports_invalid = state.reports_invalid.saturating_add(1);
            state.allocations.insert(allocation_id, allocation);
            return RequestResolution::Blocked;
        }
    }

    match allocation.stage {
        Stage::Primary => {
            if primary_is_good_enough(&allocation) {
                let (decision, cache_update) = primary_decision(&allocation);
                let saved = estimated_attempts_saved_by_primary(&allocation);
                state.primary_accepted = state.primary_accepted.saturating_add(1);
                state.estimated_probe_attempts_saved =
                    state.estimated_probe_attempts_saved.saturating_add(saved);
                let selection =
                    record_final_decision(&mut state, allocation, decision, cache_update, None);
                RequestResolution::Selected(selection)
            } else {
                transition_to_expanded(&mut allocation);
                state.expansions_triggered = state.expansions_triggered.saturating_add(1);
                let dispatch = offer_dispatch(&allocation);
                state.allocations.insert(allocation_id, allocation);
                RequestResolution::Expanded(dispatch)
            }
        }
        Stage::Eager => {
            let (decision, cache_update, fallback) = score_terminal_allocation(
                &mut state,
                &allocation,
                Stage::Eager,
                DecisionReason::ExpandedBestScore,
            );
            let selection =
                record_final_decision(&mut state, allocation, decision, cache_update, fallback);
            RequestResolution::Selected(selection)
        }
        Stage::Expanded => {
            // Expanded reports normally use the dedicated top-level message.
            // Accepting the same report here makes retry ordering idempotent.
            if expanded_ready(&allocation) {
                let (decision, cache_update, fallback) = score_terminal_allocation(
                    &mut state,
                    &allocation,
                    Stage::Expanded,
                    DecisionReason::ExpandedBestScore,
                );
                let selection =
                    record_final_decision(&mut state, allocation, decision, cache_update, fallback);
                RequestResolution::Selected(selection)
            } else {
                state.allocations.insert(allocation_id, allocation);
                RequestResolution::Pending
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ReportAccounting {
    Accepted { role: EndpointRole, stage: Stage },
    Duplicate,
    Late,
    Invalid,
    BindingMismatch,
    StageMismatch,
}

#[derive(Clone, Copy)]
enum FallbackAccounting {
    LegacyFallback,
    ProbeFailure,
    ManualOverride,
    InvalidReport,
    ReportLate,
}

pub(crate) fn handle_stage_report(
    source_route: SocketAddr,
    source_ip: IpAddr,
    report: RelayProbeReport,
) -> StageReportResolution {
    let active = starry_config::active_snapshot();
    let Some(config) = active
        .config
        .as_ref()
        .map(|value| value.relay_quality.clone())
    else {
        return StageReportResolution::Rejected;
    };
    if !config.enabled || report.allocation_id.len() != 16 {
        return StageReportResolution::Rejected;
    }
    let allocation_id = report.allocation_id.to_vec();
    let source_route = normalize_socket(source_route);
    let source_ip = normalize_ip(source_ip);
    let Some(role) = EndpointRole::from_wire(report.endpoint_role) else {
        if let Ok(mut state) = STATE.write() {
            state.reports_invalid = state.reports_invalid.saturating_add(1);
        }
        return StageReportResolution::Rejected;
    };
    let mut state = match STATE.write() {
        Ok(state) => state,
        Err(_) => return StageReportResolution::Rejected,
    };
    cleanup(&mut state, &config);

    if let Some(finalized) = state.finalized_allocations.get(&allocation_id).cloned() {
        if finalized.config_generation == active.generation
            && finalized.stage.wire() == report.stage
            && finalized.stage_token.as_slice() == report.stage_token.as_ref()
            && finalized_source_matches(&finalized, source_route, source_ip, role)
        {
            state.reports_duplicate = state.reports_duplicate.saturating_add(1);
            return StageReportResolution::Dispatch(finalized_decision_dispatch(&finalized));
        }
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return StageReportResolution::Rejected;
    }

    let Some(mut allocation) = state.allocations.remove(&allocation_id) else {
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return StageReportResolution::Rejected;
    };
    if allocation.config_generation != active.generation
        || !source_matches(&allocation, source_route, source_ip, role)
    {
        state.allocations.insert(allocation_id, allocation);
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return StageReportResolution::Rejected;
    }

    let validated = match validate_report_until(&report, &allocation, role, Instant::now()) {
        Ok(validated) => validated,
        Err(ReportRejection::StageMismatch) => {
            state.reports_stage_mismatch = state.reports_stage_mismatch.saturating_add(1);
            state.allocations.insert(allocation_id, allocation);
            return StageReportResolution::Rejected;
        }
        Err(ReportRejection::Invalid) => {
            state.reports_invalid = state.reports_invalid.saturating_add(1);
            state.allocations.insert(allocation_id, allocation);
            return StageReportResolution::Rejected;
        }
        Err(ReportRejection::Late) => {
            state.reports_late = state.reports_late.saturating_add(1);
            state.stage_timeouts = state.stage_timeouts.saturating_add(1);
            let stage = allocation.stage;
            let (decision, cache_update, fallback) = if allocation
                .reports
                .get(stage, EndpointRole::Controller)
                .is_some()
            {
                score_terminal_allocation(&mut state, &allocation, stage, DecisionReason::Partial)
            } else {
                (
                    fallback_decision(&allocation, stage, DecisionReason::ProbeFailure),
                    None,
                    Some(FallbackAccounting::ReportLate),
                )
            };
            let selection =
                record_final_decision(&mut state, allocation, decision, cache_update, fallback);
            let finalized = state
                .finalized_allocations
                .get(selection.decision.allocation_id.as_ref())
                .cloned();
            return finalized
                .map(|value| StageReportResolution::Dispatch(finalized_decision_dispatch(&value)))
                .unwrap_or(StageReportResolution::Rejected);
        }
    };

    match store_report(
        allocation.reports.slot_mut(allocation.stage, role),
        validated,
    ) {
        StoreReportResult::Accepted => account_report(
            &mut state,
            ReportAccounting::Accepted {
                role,
                stage: allocation.stage,
            },
        ),
        StoreReportResult::Duplicate => {
            state.reports_duplicate = state.reports_duplicate.saturating_add(1)
        }
        StoreReportResult::Conflict => {
            state.reports_invalid = state.reports_invalid.saturating_add(1);
            state.allocations.insert(allocation_id, allocation);
            return StageReportResolution::Rejected;
        }
    }

    let terminal = match allocation.stage {
        Stage::Expanded => expanded_ready(&allocation),
        Stage::Eager => eager_ready(&allocation),
        Stage::Primary => false,
    };
    if !terminal {
        state.allocations.insert(allocation_id, allocation);
        return StageReportResolution::Pending;
    }
    let stage = allocation.stage;
    let reason = if stage_report_count(&allocation, stage) < 2 {
        DecisionReason::Partial
    } else {
        DecisionReason::ExpandedBestScore
    };
    let (decision, cache_update, fallback) =
        score_terminal_allocation(&mut state, &allocation, stage, reason);
    let selection = record_final_decision(&mut state, allocation, decision, cache_update, fallback);
    state
        .finalized_allocations
        .get(selection.decision.allocation_id.as_ref())
        .cloned()
        .map(|value| StageReportResolution::Dispatch(finalized_decision_dispatch(&value)))
        .unwrap_or(StageReportResolution::Rejected)
}

pub(crate) fn cancel_allocation(
    source_route: SocketAddr,
    source_ip: IpAddr,
    cancel: RelayQualityCancel,
) -> Option<StageDispatch> {
    let config = starry_config::snapshot()?.relay_quality.clone();
    if !config.enabled
        || cancel.protocol_version != PROTOCOL_VERSION
        || cancel.allocation_id.len() != 16
        || !matches!(
            cancel.reason_code,
            CANCEL_P2P_SUCCEEDED | CANCEL_CLIENT_ABORT
        )
    {
        return None;
    }
    let role = EndpointRole::from_wire(cancel.endpoint_role)?;
    let allocation_id = cancel.allocation_id.to_vec();
    let mut state = STATE.write().ok()?;
    cleanup(&mut state, &config);
    let allocation = state.allocations.get(&allocation_id)?;
    if allocation.stage.wire() != cancel.stage
        || allocation.stage_token.as_slice() != cancel.stage_token.as_ref()
        || !source_matches(
            allocation,
            normalize_socket(source_route),
            normalize_ip(source_ip),
            role,
        )
    {
        state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1);
        return None;
    }
    let allocation = state.allocations.remove(&allocation_id)?;
    state.routes.retain(|_, value| value != &allocation_id);
    if cancel.reason_code == CANCEL_P2P_SUCCEEDED {
        state.p2p_cancellations = state.p2p_cancellations.saturating_add(1);
        state.estimated_probe_attempts_saved = state
            .estimated_probe_attempts_saved
            .saturating_add(estimated_attempts_saved_by_cancel(&allocation));
    }
    Some(cancel_dispatch(&allocation, cancel))
}

pub(crate) fn decision_for_response(uuid: &str, source_ip: IpAddr) -> Option<RelayQualityDecision> {
    if uuid.is_empty() {
        return None;
    }
    let config = starry_config::snapshot()?.relay_quality.clone();
    let mut state = STATE.write().ok()?;
    cleanup(&mut state, &config);
    state
        .decisions
        .get(&DecisionKey {
            uuid: uuid.to_owned(),
            target_ip: normalize_ip(source_ip),
        })
        .map(|record| record.decision.clone())
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    let config = starry_config::snapshot()
        .map(|config| config.relay_quality.clone())
        .unwrap_or_default();
    let strategy = match config.strategy {
        RelayQualityStrategyConfig::Adaptive => "adaptive",
        RelayQualityStrategyConfig::Eager => "eager",
    }
    .to_owned();
    let Ok(mut state) = STATE.write() else {
        return RuntimeSnapshot {
            protocol_version: PROTOCOL_VERSION,
            strategy,
            enabled: config.enabled,
            active_allocations: 0,
            cached_network_pairs: 0,
            pending_decisions: 0,
            offers_created: 0,
            offers_skipped: 0,
            offer_skip_reasons: OfferSkipCounters::default(),
            peer_reports_accepted: 0,
            controller_reports_accepted: 0,
            reports_accepted: 0,
            reports_duplicate: 0,
            reports_stage_mismatch: 0,
            reports_late: 0,
            reports_invalid: 0,
            reports_binding_mismatch: 0,
            decisions_created: 0,
            fallback_decisions: 0,
            fallback_reasons: FallbackReasonCounters::default(),
            cache_hits: 0,
            hysteresis_decisions: 0,
            primary_probes: 0,
            primary_accepted: 0,
            expansions_triggered: 0,
            p2p_cancellations: 0,
            estimated_probe_attempts_saved: 0,
            expanded_decisions: 0,
            stage_timeouts: 0,
            relay_selections: HashMap::new(),
            relay_selection_overflow: 0,
        };
    };
    cleanup(&mut state, &config);
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        strategy,
        enabled: config.enabled,
        active_allocations: state.allocations.len(),
        cached_network_pairs: state.cache.len(),
        pending_decisions: state.decisions.len(),
        offers_created: state.offers_created,
        offers_skipped: state.offers_skipped,
        offer_skip_reasons: state.offer_skip_reasons.clone(),
        peer_reports_accepted: state.peer_reports_accepted,
        controller_reports_accepted: state.controller_reports_accepted,
        reports_accepted: state.reports_accepted,
        reports_duplicate: state.reports_duplicate,
        reports_stage_mismatch: state.reports_stage_mismatch,
        reports_late: state.reports_late,
        reports_invalid: state.reports_invalid,
        reports_binding_mismatch: state.reports_binding_mismatch,
        decisions_created: state.decisions_created,
        fallback_decisions: state.fallback_decisions,
        fallback_reasons: state.fallback_reasons.clone(),
        cache_hits: state.cache_hits,
        hysteresis_decisions: state.hysteresis_decisions,
        primary_probes: state.primary_probes,
        primary_accepted: state.primary_accepted,
        expansions_triggered: state.expansions_triggered,
        p2p_cancellations: state.p2p_cancellations,
        estimated_probe_attempts_saved: state.estimated_probe_attempts_saved,
        expanded_decisions: state.expanded_decisions,
        stage_timeouts: state.stage_timeouts,
        relay_selections: state.relay_selections.clone(),
        relay_selection_overflow: state.relay_selection_overflow,
    }
}

pub(crate) fn finalize_expired() -> Vec<StageDispatch> {
    let active = starry_config::active_snapshot();
    let Some(config) = active
        .config
        .as_ref()
        .map(|value| value.relay_quality.clone())
    else {
        return Vec::new();
    };
    if !config.enabled {
        return Vec::new();
    }
    let Ok(mut state) = STATE.write() else {
        return Vec::new();
    };
    cleanup(&mut state, &config);
    let now = Instant::now();
    let expired = state
        .allocations
        .iter()
        .filter(|(_, allocation)| {
            allocation.pending_uuid.is_some()
                && (now > allocation.stage_deadline || now > allocation.total_deadline)
        })
        .take(MAX_DEADLINE_DISPATCHES_PER_TICK)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let mut dispatches = Vec::with_capacity(expired.len());
    for allocation_id in expired {
        let Some(allocation) = state.allocations.remove(&allocation_id) else {
            continue;
        };
        state.stage_timeouts = state.stage_timeouts.saturating_add(1);
        let stage = allocation.stage;
        let (decision, cache_update, fallback) = if allocation
            .reports
            .get(stage, EndpointRole::Controller)
            .is_some()
        {
            score_terminal_allocation(&mut state, &allocation, stage, DecisionReason::Partial)
        } else {
            (
                fallback_decision(&allocation, stage, DecisionReason::ProbeFailure),
                None,
                Some(FallbackAccounting::ProbeFailure),
            )
        };
        let selection =
            record_final_decision(&mut state, allocation, decision, cache_update, fallback);
        if let Some(finalized) = state
            .finalized_allocations
            .get(selection.decision.allocation_id.as_ref())
        {
            dispatches.push(finalized_decision_dispatch(finalized));
        }
    }
    dispatches
}

fn empty_response_context() -> ResponseContext {
    ResponseContext {
        offer: None,
        peer_report: None,
    }
}

fn wire_candidate(view: &RelayRuntimeView, websocket: bool) -> RelayQualityCandidate {
    // Detailed load and the HBBS-to-HBBR latency are trusted server-side
    // inputs. They are deliberately not copied into the public client offer.
    let probe_url = if websocket {
        view.websocket.url.clone().unwrap_or_default()
    } else {
        format!("tcp://{}", view.id)
    };
    RelayQualityCandidate {
        relay_server: view.id.clone(),
        probe_url,
        ..Default::default()
    }
}

fn offer_for(allocation: &Allocation, role: EndpointRole) -> RelayQualityOffer {
    let candidates = stage_candidate_indices(allocation.stage, allocation.candidates.len())
        .into_iter()
        .filter_map(|index| allocation.candidates.get(index))
        .map(|candidate| match role {
            EndpointRole::Controller => candidate.controller_wire.clone(),
            EndpointRole::Target => candidate.target_wire.clone(),
        })
        .collect();
    RelayQualityOffer {
        protocol_version: PROTOCOL_VERSION,
        allocation_id: allocation.allocation_id.clone().into(),
        fallback_relay: allocation.fallback_relay.clone(),
        candidates,
        probe_samples: stage_samples(allocation),
        probe_interval_ms: allocation.config.probe_interval_ms,
        report_timeout_ms: allocation.config.report_timeout_ms,
        probe_timeout_ms: allocation.config.probe_timeout_ms,
        strategy: match allocation.config.strategy {
            RelayQualityStrategyConfig::Adaptive => STRATEGY_ADAPTIVE,
            RelayQualityStrategyConfig::Eager => STRATEGY_EAGER,
        },
        stage: allocation.stage.wire(),
        stage_token: allocation.stage_token.clone().into(),
        stage_deadline_unix_ms: allocation.stage_deadline_unix_ms,
        total_deadline_unix_ms: allocation.total_deadline_unix_ms,
        primary_relay: allocation.primary_relay.clone(),
        p2p_probe_grace_ms: if allocation.stage == Stage::Primary {
            allocation.config.p2p_probe_grace_ms
        } else {
            0
        },
        ..Default::default()
    }
}

fn stage_candidate_indices(stage: Stage, candidate_count: usize) -> Vec<usize> {
    match stage {
        Stage::Primary => (candidate_count > 0).then_some(0).into_iter().collect(),
        Stage::Expanded => (1..candidate_count).collect(),
        Stage::Eager => (0..candidate_count).collect(),
    }
}

fn stage_samples(allocation: &Allocation) -> u32 {
    match allocation.stage {
        Stage::Primary => allocation.config.primary_probe_samples,
        Stage::Expanded | Stage::Eager => allocation.config.probe_samples,
    }
}

fn primary_stage_budget_ms(config: &RelayQualityConfig) -> u64 {
    let endpoint_window = u64::from(config.p2p_probe_grace_ms).saturating_add(probe_window_ms(
        config.primary_probe_samples,
        config.probe_timeout_ms,
        config.probe_interval_ms,
    ));
    endpoint_window
        .saturating_mul(2)
        .saturating_add(PRIMARY_SIGNAL_MARGIN_MS)
}

fn probe_window_ms(samples: u32, timeout_ms: u32, interval_ms: u32) -> u64 {
    u64::from(samples)
        .saturating_mul(u64::from(timeout_ms))
        .saturating_add(u64::from(samples.saturating_sub(1)).saturating_mul(u64::from(interval_ms)))
}

fn checked_deadline(created: Instant, milliseconds: u64) -> Instant {
    created
        .checked_add(Duration::from_millis(milliseconds))
        .unwrap_or(created)
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn fresh_stage_token() -> Vec<u8> {
    uuid::Uuid::now_v7().as_bytes().to_vec()
}

fn validate_report_until(
    report: &RelayProbeReport,
    allocation: &Allocation,
    role: EndpointRole,
    received_at: Instant,
) -> Result<ValidatedReport, ReportRejection> {
    if report.protocol_version != PROTOCOL_VERSION
        || report.allocation_id.as_ref() != allocation.allocation_id.as_slice()
    {
        return Err(ReportRejection::Invalid);
    }
    if report.stage != allocation.stage.wire()
        || report.stage_token.as_ref() != allocation.stage_token.as_slice()
        || report.endpoint_role != role.wire()
    {
        return Err(ReportRejection::StageMismatch);
    }
    if received_at > allocation.stage_deadline || received_at > allocation.total_deadline {
        return Err(ReportRejection::Late);
    }
    validate_report(report, allocation, role).map_err(|()| ReportRejection::Invalid)
}

fn validate_report(
    report: &RelayProbeReport,
    allocation: &Allocation,
    role: EndpointRole,
) -> Result<ValidatedReport, ()> {
    let expected_indices = stage_candidate_indices(allocation.stage, allocation.candidates.len());
    if report.results.len() != expected_indices.len() {
        return Err(());
    }
    let expected_samples = stage_samples(allocation);
    let mut expected = expected_indices
        .into_iter()
        .filter_map(|index| allocation.candidates.get(index))
        .map(|candidate| candidate.relay.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut metrics = HashMap::new();
    let mut wire_results = Vec::with_capacity(report.results.len());
    for result in &report.results {
        let key = result.relay_server.to_ascii_lowercase();
        if !expected.remove(&key)
            || metrics.contains_key(&key)
            || result.attempted != expected_samples
            || result.succeeded > result.attempted
            || result.rtt_ms > MAX_REPORTED_RTT_MS
            || result.jitter_ms > MAX_REPORTED_RTT_MS
            || (result.succeeded > 0 && result.rtt_ms == 0)
            || (result.succeeded < 2 && result.jitter_ms != 0)
            || (result.succeeded == 0 && (result.rtt_ms != 0 || result.jitter_ms != 0))
        {
            return Err(());
        }
        let candidate = allocation
            .candidates
            .iter()
            .find(|candidate| candidate.relay.eq_ignore_ascii_case(&result.relay_server))
            .ok_or(())?;
        let metric = Metric {
            attempted: result.attempted,
            succeeded: result.succeeded,
            rtt_ms: result.rtt_ms,
            jitter_ms: result.jitter_ms,
        };
        metrics.insert(key, metric);
        wire_results.push(RelayProbeResult {
            relay_server: candidate.relay.clone(),
            attempted: result.attempted,
            succeeded: result.succeeded,
            rtt_ms: result.rtt_ms,
            jitter_ms: result.jitter_ms,
            ..Default::default()
        });
    }
    if !expected.is_empty() {
        return Err(());
    }
    Ok(ValidatedReport {
        metrics,
        wire: RelayProbeReport {
            protocol_version: PROTOCOL_VERSION,
            allocation_id: allocation.allocation_id.clone().into(),
            results: wire_results,
            stage: allocation.stage.wire(),
            stage_token: allocation.stage_token.clone().into(),
            endpoint_role: role.wire(),
            ..Default::default()
        },
    })
}

fn store_report(slot: &mut Option<ValidatedReport>, report: ValidatedReport) -> StoreReportResult {
    match slot {
        None => {
            *slot = Some(report);
            StoreReportResult::Accepted
        }
        Some(existing) if existing.wire == report.wire => StoreReportResult::Duplicate,
        Some(_) => StoreReportResult::Conflict,
    }
}

fn account_report(state: &mut QualityState, accounting: ReportAccounting) {
    match accounting {
        ReportAccounting::Accepted { role, stage } => {
            state.reports_accepted = state.reports_accepted.saturating_add(1);
            match role {
                EndpointRole::Controller => {
                    state.controller_reports_accepted =
                        state.controller_reports_accepted.saturating_add(1)
                }
                EndpointRole::Target => {
                    state.peer_reports_accepted = state.peer_reports_accepted.saturating_add(1)
                }
            }
            if stage == Stage::Primary {
                state.primary_probes = state.primary_probes.saturating_add(1);
            }
        }
        ReportAccounting::Duplicate => {
            state.reports_duplicate = state.reports_duplicate.saturating_add(1)
        }
        ReportAccounting::Late => state.reports_late = state.reports_late.saturating_add(1),
        ReportAccounting::Invalid => {
            state.reports_invalid = state.reports_invalid.saturating_add(1)
        }
        ReportAccounting::BindingMismatch => {
            state.reports_binding_mismatch = state.reports_binding_mismatch.saturating_add(1)
        }
        ReportAccounting::StageMismatch => {
            state.reports_stage_mismatch = state.reports_stage_mismatch.saturating_add(1)
        }
    }
}

fn primary_is_good_enough(allocation: &Allocation) -> bool {
    let Some(candidate) = allocation.candidates.first() else {
        return false;
    };
    let controller = allocation
        .reports
        .primary_controller
        .as_ref()
        .and_then(|report| report.metrics.get(&candidate.relay.to_ascii_lowercase()));
    let target = allocation
        .reports
        .primary_target
        .as_ref()
        .and_then(|report| report.metrics.get(&candidate.relay.to_ascii_lowercase()));
    if !candidate_is_viable(controller, target, false) {
        return false;
    }
    let score = score_candidate(candidate, controller, target, &allocation.config);
    score.score >= allocation.config.primary_accept_score
        && [controller, target].into_iter().flatten().all(|metric| {
            loss_basis_points(metric) <= allocation.config.primary_max_loss_basis_points
        })
}

fn primary_decision(allocation: &Allocation) -> (RelayQualityDecision, Option<(CacheKey, String)>) {
    let candidate = &allocation.candidates[0];
    let key = candidate.relay.to_ascii_lowercase();
    let controller = allocation
        .reports
        .primary_controller
        .as_ref()
        .and_then(|report| report.metrics.get(&key));
    let target = allocation
        .reports
        .primary_target
        .as_ref()
        .and_then(|report| report.metrics.get(&key));
    let partial = controller.is_none() || target.is_none();
    let reason = if partial {
        DecisionReason::Partial
    } else {
        DecisionReason::PrimaryAccepted
    };
    let selected = candidate.relay.clone();
    (
        RelayQualityDecision {
            protocol_version: PROTOCOL_VERSION,
            allocation_id: allocation.allocation_id.clone().into(),
            relay_server: selected.clone(),
            scores: vec![score_candidate(
                candidate,
                controller,
                target,
                &allocation.config,
            )],
            reason: String::new(),
            fallback: false,
            reason_code: reason.code(),
            stage: STAGE_PRIMARY,
            partial,
            ..Default::default()
        },
        Some((
            cache_key(
                allocation.initiator_ip,
                allocation.target_ip,
                allocation.requirement,
            ),
            selected,
        )),
    )
}

fn transition_to_expanded(allocation: &mut Allocation) {
    allocation.stage = Stage::Expanded;
    allocation.stage_token = fresh_stage_token();
    allocation.stage_deadline = allocation.total_deadline;
    allocation.stage_deadline_unix_ms = allocation.total_deadline_unix_ms;
}

fn expanded_ready(allocation: &Allocation) -> bool {
    let controller_ready = allocation.reports.expanded_controller.is_some();
    let target_participates =
        allocation.reports.primary_target.is_some() || allocation.reports.expanded_target.is_some();
    controller_ready && (!target_participates || allocation.reports.expanded_target.is_some())
}

fn eager_ready(allocation: &Allocation) -> bool {
    let controller_ready = allocation.reports.eager_controller.is_some();
    let target_participates = allocation.reports.eager_target.is_some();
    controller_ready && (!target_participates || allocation.reports.eager_target.is_some())
}

fn stage_report_count(allocation: &Allocation, stage: Stage) -> usize {
    [EndpointRole::Controller, EndpointRole::Target]
        .into_iter()
        .filter(|role| allocation.reports.get(stage, *role).is_some())
        .count()
}

fn score_terminal_allocation(
    state: &mut QualityState,
    allocation: &Allocation,
    stage: Stage,
    default_reason: DecisionReason,
) -> (
    RelayQualityDecision,
    Option<(CacheKey, String)>,
    Option<FallbackAccounting>,
) {
    let mut scored = allocation
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let target_required = terminal_stage_requires_target(allocation, stage);
            let controller = terminal_metric(
                allocation,
                stage,
                EndpointRole::Controller,
                index,
                candidate,
            );
            let target = terminal_metric(allocation, stage, EndpointRole::Target, index, candidate);
            (
                score_candidate(candidate, controller, target, &allocation.config),
                candidate_is_viable(controller, target, target_required),
            )
        })
        .collect::<Vec<_>>();
    let Some(mut selected_index) = scored
        .iter()
        .enumerate()
        .filter(|(_, (_, viable))| *viable)
        .max_by_key(|(index, (score, _))| (score.score, std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
    else {
        let mut decision = fallback_decision(allocation, stage, DecisionReason::ProbeFailure);
        decision.scores = sorted_scores(scored.into_iter().map(|(score, _)| score).collect());
        return (decision, None, Some(FallbackAccounting::ProbeFailure));
    };

    let key = cache_key(
        allocation.initiator_ip,
        allocation.target_ip,
        allocation.requirement,
    );
    let mut reason = default_reason;
    if let Some(cached) = state.cache.get(&key) {
        if cached.created.elapsed() <= Duration::from_secs(allocation.config.cache_ttl_seconds) {
            if let Some((index, (previous, _))) =
                scored.iter().enumerate().find(|(_, (score, viable))| {
                    *viable && score.relay_server.eq_ignore_ascii_case(&cached.relay)
                })
            {
                state.cache_hits = state.cache_hits.saturating_add(1);
                let best = scored[selected_index].0.score;
                if previous
                    .score
                    .saturating_add(allocation.config.hysteresis_basis_points)
                    >= best
                {
                    selected_index = index;
                    reason = DecisionReason::Hysteresis;
                    state.hysteresis_decisions = state.hysteresis_decisions.saturating_add(1);
                }
            }
        }
    }
    let selected = scored[selected_index].0.relay_server.clone();
    let partial = stage_report_count(allocation, stage) < 2;
    let scores = sorted_scores(scored.drain(..).map(|(score, _)| score).collect());
    (
        RelayQualityDecision {
            protocol_version: PROTOCOL_VERSION,
            allocation_id: allocation.allocation_id.clone().into(),
            relay_server: selected.clone(),
            scores,
            reason: String::new(),
            fallback: false,
            reason_code: reason.code(),
            stage: stage.wire(),
            partial,
            ..Default::default()
        },
        Some((key, selected)),
        None,
    )
}

fn terminal_metric<'a>(
    allocation: &'a Allocation,
    terminal_stage: Stage,
    role: EndpointRole,
    candidate_index: usize,
    candidate: &CandidateState,
) -> Option<&'a Metric> {
    let report_stage = if terminal_stage == Stage::Expanded && candidate_index == 0 {
        Stage::Primary
    } else {
        terminal_stage
    };
    allocation
        .reports
        .get(report_stage, role)
        .and_then(|report| report.metrics.get(&candidate.relay.to_ascii_lowercase()))
}

fn terminal_stage_requires_target(allocation: &Allocation, stage: Stage) -> bool {
    match stage {
        Stage::Primary => allocation.reports.primary_target.is_some(),
        Stage::Expanded => {
            allocation.reports.primary_target.is_some()
                || allocation.reports.expanded_target.is_some()
        }
        Stage::Eager => allocation.reports.eager_target.is_some(),
    }
}

fn candidate_is_viable(
    controller: Option<&Metric>,
    target: Option<&Metric>,
    target_required: bool,
) -> bool {
    controller
        .map(|metric| metric.succeeded > 0)
        .unwrap_or(false)
        && target
            .map(|metric| metric.succeeded > 0)
            .unwrap_or(!target_required)
}

fn score_candidate(
    candidate: &CandidateState,
    controller: Option<&Metric>,
    target: Option<&Metric>,
    config: &RelayQualityConfig,
) -> RelayQualityScore {
    let mut rtts = Vec::with_capacity(2);
    let mut jitters = Vec::with_capacity(2);
    let mut losses = Vec::with_capacity(2);
    for metric in [controller, target].into_iter().flatten() {
        losses.push(loss_basis_points(metric));
        if metric.succeeded > 0 {
            rtts.push(metric.rtt_ms);
            jitters.push(metric.jitter_ms);
        }
    }
    let missing_reports =
        2_u32.saturating_sub([controller, target].iter().flatten().count() as u32);
    let effective_rtt = match rtts.as_slice() {
        [] => candidate.server_rtt_ms.unwrap_or(config.rtt_bad_ms),
        [one] => *one,
        _ => {
            let maximum = *rtts.iter().max().unwrap_or(&config.rtt_bad_ms) as u64;
            let total = rtts.iter().map(|value| u64::from(*value)).sum::<u64>();
            ((maximum.saturating_mul(2).saturating_add(total)) / 4).min(u64::from(u32::MAX)) as u32
        }
    };
    let jitter = jitters.into_iter().max().unwrap_or_default();
    let loss = losses.into_iter().max().unwrap_or_default();
    let rtt_penalty = normalized_penalty(effective_rtt, config.rtt_bad_ms);
    let jitter_penalty = normalized_penalty(jitter, config.jitter_bad_ms);
    let loss_penalty = loss.min(SCORE_MAX);
    let load_penalty = candidate
        .load_basis_points
        .unwrap_or(UNKNOWN_LOAD_PENALTY)
        .min(SCORE_MAX);
    let weights = &config.weights;
    let weighted = (u64::from(rtt_penalty) * u64::from(weights.rtt)
        + u64::from(jitter_penalty) * u64::from(weights.jitter)
        + u64::from(loss_penalty) * u64::from(weights.loss)
        + u64::from(load_penalty) * u64::from(weights.load))
        / u64::from(SCORE_MAX);
    let missing = u64::from(missing_reports)
        .saturating_mul(u64::from(config.missing_report_penalty_basis_points));
    let total_penalty = weighted.saturating_add(missing).min(u64::from(SCORE_MAX)) as u32;
    RelayQualityScore {
        relay_server: candidate.relay.clone(),
        score: SCORE_MAX.saturating_sub(total_penalty),
        rtt_penalty,
        jitter_penalty,
        loss_penalty,
        load_penalty,
        missing_reports,
        ..Default::default()
    }
}

fn sorted_scores(mut scores: Vec<RelayQualityScore>) -> Vec<RelayQualityScore> {
    scores.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.relay_server.cmp(&right.relay_server))
    });
    scores
}

fn fallback_decision(
    allocation: &Allocation,
    stage: Stage,
    reason: DecisionReason,
) -> RelayQualityDecision {
    RelayQualityDecision {
        protocol_version: PROTOCOL_VERSION,
        allocation_id: allocation.allocation_id.clone().into(),
        relay_server: allocation.fallback_relay.clone(),
        reason: String::new(),
        fallback: true,
        reason_code: reason.code(),
        stage: stage.wire(),
        partial: true,
        ..Default::default()
    }
}

fn record_final_decision(
    state: &mut QualityState,
    allocation: Allocation,
    decision: RelayQualityDecision,
    cache_update: Option<(CacheKey, String)>,
    fallback: Option<FallbackAccounting>,
) -> RelaySelection {
    let allocation_id = allocation.allocation_id.clone();
    state.routes.retain(|_, value| value != &allocation_id);
    if let Some((key, relay)) = cache_update {
        if !state.cache.contains_key(&key) && state.cache.len() >= allocation.config.max_allocations
        {
            remove_oldest_cache_entry(state);
        }
        state.cache.insert(
            key,
            CachedSelection {
                relay,
                created: Instant::now(),
            },
        );
    }
    state.decisions_created = state.decisions_created.saturating_add(1);
    if allocation.stage == Stage::Expanded {
        state.expanded_decisions = state.expanded_decisions.saturating_add(1);
    }
    if decision.fallback {
        state.fallback_decisions = state.fallback_decisions.saturating_add(1);
    }
    if let Some(reason) = fallback {
        let counter = match reason {
            FallbackAccounting::LegacyFallback => &mut state.fallback_reasons.legacy_fallback,
            FallbackAccounting::ProbeFailure => &mut state.fallback_reasons.probe_failure,
            FallbackAccounting::ManualOverride => &mut state.fallback_reasons.manual_override,
            FallbackAccounting::InvalidReport => &mut state.fallback_reasons.invalid_report,
            FallbackAccounting::ReportLate => &mut state.fallback_reasons.report_late,
        };
        *counter = counter.saturating_add(1);
    }
    record_relay_selection(state, &decision.relay_server);

    let created = Instant::now();
    if let Some(uuid) = allocation
        .pending_uuid
        .as_ref()
        .filter(|uuid| !uuid.is_empty())
    {
        let decision_key = DecisionKey {
            uuid: uuid.clone(),
            target_ip: allocation.target_ip,
        };
        if !state.decisions.contains_key(&decision_key)
            && state.decisions.len() >= allocation.config.max_allocations
        {
            remove_oldest_decision(state);
        }
        state.decisions.insert(
            decision_key,
            DecisionRecord {
                decision: decision.clone(),
                allocation_id: allocation_id.clone(),
                created,
                config_generation: allocation.config_generation,
                target_ip: allocation.target_ip,
            },
        );
        let request_key = RequestDecisionKey {
            uuid: uuid.clone(),
            initiator_ip: allocation.initiator_ip,
            target_ip: allocation.target_ip,
        };
        if !state.request_decisions.contains_key(&request_key)
            && state.request_decisions.len() >= allocation.config.max_allocations
        {
            remove_oldest_request_decision(state);
        }
        state.request_decisions.insert(
            request_key,
            DecisionRecord {
                decision: decision.clone(),
                allocation_id: allocation_id.clone(),
                created,
                config_generation: allocation.config_generation,
                target_ip: allocation.target_ip,
            },
        );
    }
    if !state.finalized_allocations.contains_key(&allocation_id)
        && state.finalized_allocations.len() >= allocation.config.max_allocations
    {
        remove_oldest_finalized_allocation(state);
    }
    state.finalized_allocations.insert(
        allocation_id,
        FinalizedAllocation {
            decision: decision.clone(),
            created,
            config_generation: allocation.config_generation,
            initiator_route: allocation.initiator_route,
            initiator_ip: allocation.initiator_ip,
            target_route: allocation.target_route,
            target_ip: allocation.target_ip,
            target_id: allocation.target_id,
            target_websocket: allocation.target_websocket,
            stage: allocation.stage,
            stage_token: allocation.stage_token,
        },
    );
    RelaySelection {
        decision,
        target_ip: allocation.target_ip,
        config_generation: allocation.config_generation,
    }
}

fn record_relay_selection(state: &mut QualityState, relay: &str) {
    if let Some(count) = state.relay_selections.get_mut(relay) {
        *count = count.saturating_add(1);
    } else if state.relay_selections.len() < MAX_RELAY_COUNTER_DIMENSION {
        state.relay_selections.insert(relay.to_owned(), 1);
    } else {
        state.relay_selection_overflow = state.relay_selection_overflow.saturating_add(1);
    }
}

fn offer_dispatch(allocation: &Allocation) -> StageDispatch {
    StageDispatch {
        controller_route: allocation.initiator_route,
        target_route: allocation.target_route,
        target_id: allocation.target_id.clone(),
        target_websocket: allocation.target_websocket,
        controller: DispatchPayload::Offer(offer_for(allocation, EndpointRole::Controller)),
        target: DispatchPayload::Offer(offer_for(allocation, EndpointRole::Target)),
    }
}

fn finalized_decision_dispatch(allocation: &FinalizedAllocation) -> StageDispatch {
    StageDispatch {
        controller_route: allocation.initiator_route,
        target_route: allocation.target_route,
        target_id: allocation.target_id.clone(),
        target_websocket: allocation.target_websocket,
        controller: DispatchPayload::Decision(allocation.decision.clone()),
        target: DispatchPayload::Decision(allocation.decision.clone()),
    }
}

fn cancel_dispatch(allocation: &Allocation, cancel: RelayQualityCancel) -> StageDispatch {
    StageDispatch {
        controller_route: allocation.initiator_route,
        target_route: allocation.target_route,
        target_id: allocation.target_id.clone(),
        target_websocket: allocation.target_websocket,
        controller: DispatchPayload::Cancel(cancel.clone()),
        target: DispatchPayload::Cancel(cancel),
    }
}

fn source_matches(
    allocation: &Allocation,
    route: SocketAddr,
    ip: IpAddr,
    role: EndpointRole,
) -> bool {
    match role {
        EndpointRole::Controller => {
            allocation.initiator_route == route && allocation.initiator_ip == ip
        }
        EndpointRole::Target => allocation.target_route == route && allocation.target_ip == ip,
    }
}

fn initial_response_source_matches(
    allocation: &Allocation,
    route: SocketAddr,
    ip: IpAddr,
    target_id: &str,
) -> bool {
    let route = normalize_socket(route);
    let ip = normalize_ip(ip);
    if allocation.target_id != target_id {
        return false;
    }
    if source_matches(allocation, route, ip, EndpointRole::Target) {
        return true;
    }
    !allocation.target_websocket
        && allocation.target_ip == ip
        && normalize_ip(route.ip()) == normalize_ip(allocation.target_route.ip())
}

fn finalized_source_matches(
    allocation: &FinalizedAllocation,
    route: SocketAddr,
    ip: IpAddr,
    role: EndpointRole,
) -> bool {
    match role {
        EndpointRole::Controller => {
            allocation.initiator_route == route && allocation.initiator_ip == ip
        }
        EndpointRole::Target => allocation.target_route == route && allocation.target_ip == ip,
    }
}

fn estimated_attempts_saved_by_primary(allocation: &Allocation) -> u64 {
    let remaining = allocation.candidates.len().saturating_sub(1) as u64;
    remaining
        .saturating_mul(u64::from(allocation.config.probe_samples))
        .saturating_mul(2)
}

fn estimated_attempts_saved_by_cancel(allocation: &Allocation) -> u64 {
    let candidates = stage_candidate_indices(allocation.stage, allocation.candidates.len()).len();
    let reports = stage_report_count(allocation, allocation.stage).min(2);
    (candidates as u64)
        .saturating_mul(u64::from(stage_samples(allocation)))
        .saturating_mul((2_usize.saturating_sub(reports)) as u64)
}

fn loss_basis_points(metric: &Metric) -> u32 {
    metric
        .attempted
        .saturating_sub(metric.succeeded)
        .saturating_mul(SCORE_MAX)
        / metric.attempted.max(1)
}

fn normalized_penalty(value: u32, bad: u32) -> u32 {
    (u64::from(value).saturating_mul(u64::from(SCORE_MAX)) / u64::from(bad.max(1)))
        .min(u64::from(SCORE_MAX)) as u32
}

fn cache_key(client_a: IpAddr, client_b: IpAddr, requirement: RelayRequirement) -> CacheKey {
    let mut endpoints = [network_prefix(client_a), network_prefix(client_b)];
    endpoints.sort();
    CacheKey {
        endpoint_a: endpoints[0].clone(),
        endpoint_b: endpoints[1].clone(),
        requirement: match requirement {
            RelayRequirement::NativeOnly => 0,
            RelayRequirement::WebSocketOnly => 1,
            RelayRequirement::Mixed => 2,
        },
    }
}

fn network_prefix(ip: IpAddr) -> NetworkPrefix {
    match normalize_ip(ip) {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            NetworkPrefix::V4([octets[0], octets[1], octets[2]])
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            NetworkPrefix::V6([
                octets[0], octets[1], octets[2], octets[3], octets[4], octets[5], octets[6],
            ])
        }
    }
}

fn normalize_socket(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(normalize_ip(address.ip()), address.port())
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

fn cleanup(state: &mut QualityState, config: &RelayQualityConfig) {
    let allocation_ttl = Duration::from_secs(config.allocation_ttl_seconds);
    let expired = state
        .allocations
        .iter()
        .filter(|(_, allocation)| allocation.created.elapsed() > allocation_ttl)
        .map(|(id, allocation)| (id.clone(), allocation.pending_uuid.is_some()))
        .collect::<Vec<_>>();
    let timed_out = expired.iter().filter(|(_, requested)| *requested).count();
    let expired_ids = expired
        .into_iter()
        .map(|(id, _)| id)
        .collect::<HashSet<_>>();
    if !expired_ids.is_empty() {
        state.stage_timeouts = state
            .stage_timeouts
            .saturating_add(timed_out.min(u64::MAX as usize) as u64);
        state.allocations.retain(|id, _| !expired_ids.contains(id));
        state.routes.retain(|_, id| !expired_ids.contains(id));
    }
    let cache_ttl = Duration::from_secs(config.cache_ttl_seconds);
    state
        .cache
        .retain(|_, entry| entry.created.elapsed() <= cache_ttl);
    state
        .decisions
        .retain(|_, entry| entry.created.elapsed() <= allocation_ttl);
    state
        .request_decisions
        .retain(|_, entry| entry.created.elapsed() <= allocation_ttl);
    state
        .finalized_allocations
        .retain(|_, entry| entry.created.elapsed() <= allocation_ttl);
}

fn ensure_allocation_capacity(state: &mut QualityState, limit: usize) {
    while state.allocations.len() >= limit.max(1) {
        remove_oldest_allocation(state);
    }
}

fn remove_oldest_allocation(state: &mut QualityState) {
    let Some(oldest) = state
        .allocations
        .iter()
        .max_by_key(|(_, allocation)| allocation.created.elapsed())
        .map(|(id, _)| id.clone())
    else {
        return;
    };
    state.allocations.remove(&oldest);
    state.routes.retain(|_, id| id != &oldest);
}

fn remove_oldest_cache_entry(state: &mut QualityState) {
    let Some(oldest) = state
        .cache
        .iter()
        .max_by_key(|(_, entry)| entry.created.elapsed())
        .map(|(key, _)| key.clone())
    else {
        return;
    };
    state.cache.remove(&oldest);
}

fn remove_oldest_decision(state: &mut QualityState) {
    let Some(oldest) = state
        .decisions
        .iter()
        .max_by_key(|(_, entry)| entry.created.elapsed())
        .map(|(key, _)| key.clone())
    else {
        return;
    };
    state.decisions.remove(&oldest);
}

fn remove_oldest_request_decision(state: &mut QualityState) {
    let Some(oldest) = state
        .request_decisions
        .iter()
        .max_by_key(|(_, entry)| entry.created.elapsed())
        .map(|(key, _)| key.clone())
    else {
        return;
    };
    state.request_decisions.remove(&oldest);
}

fn remove_oldest_finalized_allocation(state: &mut QualityState) {
    let Some(oldest) = state
        .finalized_allocations
        .iter()
        .max_by_key(|(_, entry)| entry.created.elapsed())
        .map(|(id, _)| id.clone())
    else {
        return;
    };
    state.finalized_allocations.remove(&oldest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn test_config(strategy: RelayQualityStrategyConfig) -> RelayQualityConfig {
        RelayQualityConfig {
            enabled: true,
            strategy,
            missing_report_penalty_basis_points: 500,
            ..RelayQualityConfig::default()
        }
    }

    fn candidate(relay: &str, load: u32) -> CandidateState {
        CandidateState {
            relay: relay.to_owned(),
            server_rtt_ms: Some(20),
            load_basis_points: Some(load),
            controller_wire: RelayQualityCandidate {
                relay_server: relay.to_owned(),
                probe_url: format!("tcp://{relay}"),
                ..Default::default()
            },
            target_wire: RelayQualityCandidate {
                relay_server: relay.to_owned(),
                probe_url: format!("wss://{relay}/ws/relay"),
                ..Default::default()
            },
        }
    }

    fn allocation(
        strategy: RelayQualityStrategyConfig,
        stage: Stage,
        candidates: Vec<CandidateState>,
    ) -> Allocation {
        let config = test_config(strategy);
        let now = Instant::now();
        Allocation {
            created: now,
            total_deadline: now + Duration::from_secs(15),
            total_deadline_unix_ms: 15_000,
            stage_deadline: now + Duration::from_secs(7),
            stage_deadline_unix_ms: 7_000,
            config_generation: 7,
            initiator_route: "192.0.2.10:40000".parse().unwrap(),
            initiator_ip: "192.0.2.10".parse().unwrap(),
            target_route: "198.51.100.20:50000".parse().unwrap(),
            target_ip: "198.51.100.20".parse().unwrap(),
            target_id: "target-test-id".to_owned(),
            target_websocket: true,
            requirement: RelayRequirement::Mixed,
            fallback_relay: candidates[0].relay.clone(),
            primary_relay: candidates[0].relay.clone(),
            allocation_id: vec![7; 16],
            stage_token: vec![8; 16],
            stage,
            candidates,
            config,
            reports: StageReports::default(),
            pending_uuid: Some("session-test".to_owned()),
        }
    }

    fn metric(attempted: u32, succeeded: u32, rtt_ms: u32, jitter_ms: u32) -> Metric {
        Metric {
            attempted,
            succeeded,
            rtt_ms,
            jitter_ms,
        }
    }

    fn validated(metrics: &[(&str, Metric)]) -> ValidatedReport {
        ValidatedReport {
            metrics: metrics
                .iter()
                .map(|(relay, metric)| (relay.to_ascii_lowercase(), *metric))
                .collect(),
            ..Default::default()
        }
    }

    fn report_for(
        allocation: &Allocation,
        role: EndpointRole,
        metrics: &[(&str, Metric)],
    ) -> RelayProbeReport {
        RelayProbeReport {
            protocol_version: PROTOCOL_VERSION,
            allocation_id: allocation.allocation_id.clone().into(),
            results: metrics
                .iter()
                .map(|(relay, metric)| RelayProbeResult {
                    relay_server: (*relay).to_owned(),
                    attempted: metric.attempted,
                    succeeded: metric.succeeded,
                    rtt_ms: metric.rtt_ms,
                    jitter_ms: metric.jitter_ms,
                    ..Default::default()
                })
                .collect(),
            stage: allocation.stage.wire(),
            stage_token: allocation.stage_token.clone().into(),
            endpoint_role: role.wire(),
            ..Default::default()
        }
    }

    #[test]
    fn adaptive_primary_offer_contains_only_geo_primary_and_no_trusted_load() {
        let allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![
                candidate("relay-a:21117", 1_000),
                candidate("relay-b:21117", 2_000),
            ],
        );

        let offer = offer_for(&allocation, EndpointRole::Controller);

        assert_eq!(offer.strategy, STRATEGY_ADAPTIVE);
        assert_eq!(offer.stage, STAGE_PRIMARY);
        assert_eq!(offer.primary_relay, "relay-a:21117");
        assert_eq!(offer.candidates.len(), 1);
        assert_eq!(offer.candidates[0].relay_server, offer.primary_relay);
        assert!(offer.candidates[0].load.is_none());
        assert_eq!(offer.candidates[0].server_rtt_ms, 0);
        assert_eq!(offer.candidates[0].observed_at_unix_ms, 0);
        assert!(offer.weights.is_none());
        assert_eq!(offer.probe_samples, allocation.config.primary_probe_samples);
        assert_eq!(offer.stage_token.len(), 16);
    }

    #[test]
    fn primary_good_is_accepted_without_expansion() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![
                candidate("relay-a", 500),
                candidate("relay-b", 1_000),
                candidate("relay-c", 1_000),
            ],
        );
        let good = metric(allocation.config.primary_probe_samples, 3, 20, 2);
        allocation.reports.primary_controller = Some(validated(&[("relay-a", good)]));
        allocation.reports.primary_target = Some(validated(&[("relay-a", good)]));

        assert!(primary_is_good_enough(&allocation));
        let (decision, _) = primary_decision(&allocation);
        assert_eq!(decision.relay_server, "relay-a");
        assert_eq!(decision.reason_code, DECISION_PRIMARY_ACCEPTED);
        assert!(!decision.partial);
        assert_eq!(estimated_attempts_saved_by_primary(&allocation), 20);
    }

    #[test]
    fn either_endpoint_primary_loss_triggers_concurrent_expansion() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![
                candidate("relay-a", 500),
                candidate("relay-b", 1_000),
                candidate("relay-c", 1_000),
            ],
        );
        let samples = allocation.config.primary_probe_samples;
        allocation.reports.primary_controller =
            Some(validated(&[("relay-a", metric(samples, samples, 20, 2))]));
        allocation.reports.primary_target =
            Some(validated(&[("relay-a", metric(samples, 1, 20, 0))]));
        assert!(!primary_is_good_enough(&allocation));

        let old_token = allocation.stage_token.clone();
        transition_to_expanded(&mut allocation);
        let dispatch = offer_dispatch(&allocation);
        let DispatchPayload::Offer(controller) = dispatch.controller else {
            panic!("controller must receive expansion offer");
        };
        let DispatchPayload::Offer(target) = dispatch.target else {
            panic!("target must receive expansion offer");
        };
        assert_eq!(controller.stage, STAGE_EXPANDED);
        assert_eq!(target.stage, STAGE_EXPANDED);
        assert_ne!(controller.stage_token.as_ref(), old_token.as_slice());
        assert_eq!(controller.stage_token, target.stage_token);
        assert_eq!(
            controller
                .candidates
                .iter()
                .map(|candidate| candidate.relay_server.as_str())
                .collect::<Vec<_>>(),
            vec!["relay-b", "relay-c"]
        );
        assert_eq!(controller.candidates.len(), target.candidates.len());
    }

    #[test]
    fn expanded_dual_endpoint_scoring_produces_one_identical_decision() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Expanded,
            vec![
                candidate("relay-a", 500),
                candidate("relay-b", 1_000),
                candidate("relay-c", 1_000),
            ],
        );
        allocation.reports.primary_controller = Some(validated(&[(
            "relay-a",
            metric(allocation.config.primary_probe_samples, 1, 150, 0),
        )]));
        allocation.reports.primary_target = allocation.reports.primary_controller.clone();
        let samples = allocation.config.probe_samples;
        allocation.reports.expanded_controller = Some(validated(&[
            ("relay-b", metric(samples, samples, 35, 3)),
            ("relay-c", metric(samples, samples, 80, 8)),
        ]));
        allocation.reports.expanded_target = Some(validated(&[
            ("relay-b", metric(samples, samples, 40, 4)),
            ("relay-c", metric(samples, samples, 75, 7)),
        ]));
        let mut state = QualityState::default();
        let (decision, cache, fallback) = score_terminal_allocation(
            &mut state,
            &allocation,
            Stage::Expanded,
            DecisionReason::ExpandedBestScore,
        );
        assert_eq!(decision.relay_server, "relay-b");
        assert_eq!(decision.reason_code, DECISION_EXPANDED_BEST_SCORE);
        assert!(!decision.partial);
        assert!(cache.is_some());
        assert!(fallback.is_none());

        let selection = record_final_decision(&mut state, allocation, decision, cache, fallback);
        let finalized = state
            .finalized_allocations
            .get(selection.decision.allocation_id.as_ref())
            .unwrap();
        let dispatch = finalized_decision_dispatch(finalized);
        let DispatchPayload::Decision(controller) = dispatch.controller else {
            panic!("controller decision missing");
        };
        let DispatchPayload::Decision(target) = dispatch.target else {
            panic!("target decision missing");
        };
        assert_eq!(controller, target);
        assert_eq!(controller.relay_server, selection.decision.relay_server);
    }

    #[test]
    fn official_peer_single_endpoint_primary_report_has_explicit_partial_reason() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
        );
        let samples = allocation.config.primary_probe_samples;
        allocation.reports.primary_controller =
            Some(validated(&[("relay-a", metric(samples, samples, 25, 2))]));

        assert!(primary_is_good_enough(&allocation));
        let (decision, _) = primary_decision(&allocation);
        assert_eq!(decision.reason_code, DECISION_PARTIAL);
        assert!(decision.partial);
        assert_eq!(decision.relay_server, allocation.fallback_relay);
    }

    #[test]
    fn eager_compatibility_offer_keeps_all_candidates() {
        let allocation = allocation(
            RelayQualityStrategyConfig::Eager,
            Stage::Eager,
            vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
        );
        let offer = offer_for(&allocation, EndpointRole::Target);
        assert_eq!(offer.strategy, STRATEGY_EAGER);
        assert_eq!(offer.stage, STAGE_EAGER);
        assert_eq!(offer.p2p_probe_grace_ms, 0);
        assert_eq!(offer.candidates.len(), 2);
        assert_eq!(offer.probe_samples, allocation.config.probe_samples);
    }

    #[test]
    fn report_binding_rejects_wrong_stage_token_and_conflicting_duplicate() {
        let allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
        );
        let samples = allocation.config.primary_probe_samples;
        let metric = metric(samples, samples, 20, 2);
        let report = report_for(
            &allocation,
            EndpointRole::Controller,
            &[("relay-a", metric)],
        );
        assert!(validate_report_until(
            &report,
            &allocation,
            EndpointRole::Controller,
            Instant::now(),
        )
        .is_ok());

        let mut wrong = report.clone();
        wrong.stage_token = vec![9; 16].into();
        assert!(matches!(
            validate_report_until(
                &wrong,
                &allocation,
                EndpointRole::Controller,
                Instant::now(),
            ),
            Err(ReportRejection::StageMismatch)
        ));

        let validated = validate_report_until(
            &report,
            &allocation,
            EndpointRole::Controller,
            Instant::now(),
        )
        .unwrap();
        let mut slot = None;
        assert_eq!(
            store_report(&mut slot, validated.clone()),
            StoreReportResult::Accepted
        );
        assert_eq!(
            store_report(&mut slot, validated),
            StoreReportResult::Duplicate
        );
        let mut conflict = report;
        conflict.results[0].rtt_ms = 21;
        let conflict = validate_report_until(
            &conflict,
            &allocation,
            EndpointRole::Controller,
            Instant::now(),
        )
        .unwrap();
        assert_eq!(
            store_report(&mut slot, conflict),
            StoreReportResult::Conflict
        );
    }

    #[test]
    fn native_initial_response_allows_only_a_same_ip_port_change() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
        );
        allocation.target_websocket = false;
        let changed_port: SocketAddr = "198.51.100.20:50123".parse().unwrap();

        assert!(initial_response_source_matches(
            &allocation,
            changed_port,
            allocation.target_ip,
            &allocation.target_id,
        ));
        assert!(!source_matches(
            &allocation,
            changed_port,
            allocation.target_ip,
            EndpointRole::Target,
        ));
        assert!(!initial_response_source_matches(
            &allocation,
            changed_port,
            allocation.target_ip,
            "another-target",
        ));
        assert!(!initial_response_source_matches(
            &allocation,
            "203.0.113.20:50123".parse().unwrap(),
            "203.0.113.20".parse().unwrap(),
            &allocation.target_id,
        ));

        allocation.target_websocket = true;
        assert!(!initial_response_source_matches(
            &allocation,
            changed_port,
            allocation.target_ip,
            &allocation.target_id,
        ));
        assert!(initial_response_source_matches(
            &allocation,
            allocation.target_route,
            allocation.target_ip,
            &allocation.target_id,
        ));
    }

    #[test]
    fn deadline_is_independent_of_cleanup_ttl() {
        let mut allocation = allocation(
            RelayQualityStrategyConfig::Adaptive,
            Stage::Primary,
            vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
        );
        allocation.stage_deadline = Instant::now() - Duration::from_millis(1);
        let samples = allocation.config.primary_probe_samples;
        let report = report_for(
            &allocation,
            EndpointRole::Controller,
            &[("relay-a", metric(samples, samples, 20, 2))],
        );
        assert!(matches!(
            validate_report_until(
                &report,
                &allocation,
                EndpointRole::Controller,
                Instant::now(),
            ),
            Err(ReportRejection::Late)
        ));
        assert!(allocation.created.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn network_cache_key_is_symmetric_and_privacy_bounded() {
        let a: IpAddr = "192.0.2.44".parse().unwrap();
        let b: IpAddr = "2001:db8:abcd:1200::99".parse().unwrap();
        assert!(
            cache_key(a, b, RelayRequirement::NativeOnly)
                == cache_key(b, a, RelayRequirement::NativeOnly)
        );
        assert!(
            network_prefix(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)))
                == network_prefix(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 254)))
        );
        assert!(
            network_prefix(IpAddr::V6(Ipv6Addr::LOCALHOST))
                != network_prefix("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn allocation_and_deadline_dispatch_limits_are_hard_bounded() {
        let mut state = QualityState::default();
        for index in 0..3_u8 {
            let mut value = allocation(
                RelayQualityStrategyConfig::Adaptive,
                Stage::Primary,
                vec![candidate("relay-a", 500), candidate("relay-b", 1_000)],
            );
            value.allocation_id = vec![index; 16];
            state.allocations.insert(value.allocation_id.clone(), value);
            ensure_allocation_capacity(&mut state, 2);
        }
        assert!(state.allocations.len() <= 2);
        assert_eq!(MAX_DEADLINE_DISPATCHES_PER_TICK, 64);
    }
}
