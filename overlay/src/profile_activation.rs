use hbb_common::{
    bytes::Bytes,
    tokio::{
        sync::{Mutex, OwnedMutexGuard, RwLock},
        time::{Duration, Instant},
    },
};
use once_cell::sync::Lazy;
use serde_derive::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const ACTIVATION_ID_BYTES: usize = 16;
pub(crate) const ROUTE_LEASE_BYTES: usize = 32;
pub(crate) const LEASE_TTL_SECONDS: u64 = 45;
pub(crate) const BURST_WINDOW_SECONDS: u64 = 30;
pub(crate) const BURST_LIMIT: usize = 12;

const PUBLIC_KEY_MAX_BYTES: usize = 512;
const LEASE_TTL: Duration = Duration::from_secs(LEASE_TTL_SECONDS);
const RECORD_RETENTION: Duration = Duration::from_secs(15 * 60);
const BURST_WINDOW: Duration = Duration::from_secs(BURST_WINDOW_SECONDS);
const MAX_PEER_LOCKS: usize = 100_000;
const MAX_LEASES: usize = 100_000;
const MAX_V2_PEERS: usize = 100_000;
const MAX_BURST_IDENTITIES: usize = 100_000;
const MAINTENANCE_INTERVAL: usize = 256;

static REGISTRY: Lazy<Arc<Registry>> = Lazy::new(|| Arc::new(Registry::default()));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteKind {
    Native(SocketAddr),
    WebSocket { generation: u64, connection_id: u64 },
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) active_leases: usize,
    pub(crate) last_route_generation: u64,
    pub(crate) leases_issued: u64,
    pub(crate) leases_reused: u64,
    pub(crate) ready_acks: u64,
    pub(crate) fast_reregistrations: u64,
    pub(crate) renewals: u64,
    pub(crate) route_replacements: u64,
    pub(crate) deactivations: u64,
    pub(crate) disconnect_cleanups: u64,
    pub(crate) ttl_expirations: u64,
    pub(crate) invalid_requests: u64,
    pub(crate) stale_rejections: u64,
    pub(crate) rate_limited: u64,
    pub(crate) capacity_rejections: u64,
    pub(crate) lease_ttl_seconds: u64,
    pub(crate) burst_window_seconds: u64,
    pub(crate) burst_limit: usize,
}

struct LeaseRecord {
    uuid: Bytes,
    public_key: Bytes,
    activation_epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
    generation: u64,
    route: Option<RouteKind>,
    retired: bool,
    updated_at: Instant,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct IdentityKey {
    peer_id: String,
    uuid: Vec<u8>,
    public_key_sha256: [u8; 32],
}

#[derive(Default)]
struct Counters {
    leases_issued: AtomicU64,
    leases_reused: AtomicU64,
    ready_acks: AtomicU64,
    fast_reregistrations: AtomicU64,
    renewals: AtomicU64,
    route_replacements: AtomicU64,
    deactivations: AtomicU64,
    disconnect_cleanups: AtomicU64,
    ttl_expirations: AtomicU64,
    invalid_requests: AtomicU64,
    stale_rejections: AtomicU64,
    rate_limited: AtomicU64,
    capacity_rejections: AtomicU64,
}

struct Registry {
    next_generation: AtomicU64,
    peer_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    leases: RwLock<HashMap<String, LeaseRecord>>,
    v2_peers: RwLock<HashSet<String>>,
    bursts: Mutex<HashMap<IdentityKey, VecDeque<Instant>>>,
    pending_new_leases: AtomicUsize,
    maintenance_calls: AtomicUsize,
    counters: Counters,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            next_generation: AtomicU64::new(1),
            peer_locks: Mutex::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
            v2_peers: RwLock::new(HashSet::new()),
            bursts: Mutex::new(HashMap::new()),
            pending_new_leases: AtomicUsize::new(0),
            maintenance_calls: AtomicUsize::new(0),
            counters: Counters::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivationError {
    Invalid,
    RateLimited,
    Capacity,
    Stale,
}

pub(crate) struct ActivationClaim {
    registry: Arc<Registry>,
    peer_id: String,
    uuid: Bytes,
    public_key: Bytes,
    activation_epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
    generation: u64,
    previous_route: Option<RouteKind>,
    tracked: bool,
    new_lease: bool,
    fast_reregistration: bool,
    capacity_reserved: bool,
    _guard: OwnedMutexGuard<()>,
}

impl ActivationClaim {
    pub(crate) fn activation_id(&self) -> &Bytes {
        &self.activation_id
    }

    pub(crate) fn activation_epoch(&self) -> u64 {
        self.activation_epoch
    }

    pub(crate) fn route_lease(&self) -> &Bytes {
        &self.route_lease
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn previous_route(&self) -> Option<RouteKind> {
        self.previous_route
    }

    pub(crate) fn tracked(&self) -> bool {
        self.tracked
    }

    pub(crate) async fn commit(mut self, route: RouteKind) {
        if !self.tracked {
            return;
        }
        let replaced = self
            .previous_route
            .is_some_and(|previous| previous != route);
        self.registry.leases.write().await.insert(
            self.peer_id.clone(),
            LeaseRecord {
                uuid: self.uuid.clone(),
                public_key: self.public_key.clone(),
                activation_epoch: self.activation_epoch,
                activation_id: self.activation_id.clone(),
                route_lease: self.route_lease.clone(),
                generation: self.generation,
                route: Some(route),
                retired: false,
                updated_at: Instant::now(),
            },
        );
        if self.capacity_reserved {
            self.registry
                .pending_new_leases
                .fetch_sub(1, Ordering::AcqRel);
            self.capacity_reserved = false;
        }
        if self.new_lease {
            self.registry
                .counters
                .leases_issued
                .fetch_add(1, Ordering::Relaxed);
        }
        if self.fast_reregistration {
            self.registry
                .counters
                .fast_reregistrations
                .fetch_add(1, Ordering::Relaxed);
        }
        if replaced {
            self.registry
                .counters
                .route_replacements
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for ActivationClaim {
    fn drop(&mut self) {
        if self.capacity_reserved {
            self.registry
                .pending_new_leases
                .fetch_sub(1, Ordering::AcqRel);
        }
    }
}

pub(crate) struct RenewalClaim {
    registry: Arc<Registry>,
    peer_id: String,
    generation: u64,
    previous_route: Option<RouteKind>,
    tracked: bool,
    _guard: OwnedMutexGuard<()>,
}

impl RenewalClaim {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn previous_route(&self) -> Option<RouteKind> {
        self.previous_route
    }

    pub(crate) fn tracked(&self) -> bool {
        self.tracked
    }

    pub(crate) fn reject_route_change(self) {
        if self.tracked {
            self.registry
                .counters
                .stale_rejections
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) async fn commit(self, route: RouteKind) {
        if !self.tracked {
            return;
        }
        let mut leases = self.registry.leases.write().await;
        if let Some(lease) = leases
            .get_mut(&self.peer_id)
            .filter(|lease| lease.generation == self.generation && !lease.retired)
        {
            if lease.route.is_some_and(|previous| previous != route) {
                self.registry
                    .counters
                    .route_replacements
                    .fetch_add(1, Ordering::Relaxed);
            }
            lease.route = Some(route);
            lease.updated_at = Instant::now();
            self.registry
                .counters
                .renewals
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct DeactivationClaim {
    registry: Arc<Registry>,
    peer_id: String,
    activation_epoch: u64,
    generation: u64,
    route: RouteKind,
    _guard: OwnedMutexGuard<()>,
}

impl DeactivationClaim {
    pub(crate) fn route(&self) -> RouteKind {
        self.route
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) async fn commit(self, route_removed: bool) -> bool {
        if !route_removed {
            return false;
        }
        let mut leases = self.registry.leases.write().await;
        if let Some(lease) = leases.get_mut(&self.peer_id).filter(|lease| {
            lease.activation_epoch == self.activation_epoch
                && lease.generation == self.generation
                && lease.route == Some(self.route)
                && !lease.retired
        }) {
            lease.route = None;
            lease.route_lease = Bytes::new();
            lease.retired = true;
            lease.updated_at = Instant::now();
            self.registry
                .counters
                .deactivations
                .fetch_add(1, Ordering::Relaxed);
            true
        } else {
            self.registry
                .counters
                .stale_rejections
                .fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

pub(crate) async fn begin_registration(
    peer_id: &str,
    uuid: &Bytes,
    public_key: &Bytes,
    activation_epoch: u64,
    activation_id: &Bytes,
    fast_identity_verified: bool,
) -> Result<ActivationClaim, ActivationError> {
    REGISTRY
        .clone()
        .begin_registration(
            peer_id,
            uuid,
            public_key,
            activation_epoch,
            activation_id,
            fast_identity_verified,
        )
        .await
}

pub(crate) async fn begin_renewal(
    peer_id: &str,
    activation_epoch: u64,
    activation_id: &Bytes,
    route_lease: &Bytes,
    route_generation: u64,
) -> Result<RenewalClaim, ActivationError> {
    REGISTRY
        .clone()
        .begin_renewal(
            peer_id,
            activation_epoch,
            activation_id,
            route_lease,
            route_generation,
        )
        .await
}

pub(crate) async fn begin_deactivation(
    peer_id: &str,
    uuid: &Bytes,
    activation_epoch: u64,
    activation_id: &Bytes,
    route_lease: &Bytes,
    route_generation: u64,
) -> Result<DeactivationClaim, ActivationError> {
    REGISTRY
        .clone()
        .begin_deactivation(
            peer_id,
            uuid,
            activation_epoch,
            activation_id,
            route_lease,
            route_generation,
        )
        .await
}

pub(crate) async fn touch_route(peer_id: &str, route: RouteKind) -> bool {
    REGISTRY.clone().touch_route(peer_id, route).await
}

pub(crate) async fn disconnect_route(peer_id: &str, route: RouteKind) -> bool {
    REGISTRY.clone().disconnect_route(peer_id, route).await
}

pub(crate) fn disconnect_websocket_routes_now(routes: &[(String, u64, u64)]) -> usize {
    REGISTRY.disconnect_websocket_routes_now(routes)
}

pub(crate) async fn verify_active(
    peer_id: &str,
    uuid: &Bytes,
    activation_epoch: u64,
    activation_id: &Bytes,
    route_leases: &[Bytes],
) -> bool {
    REGISTRY
        .clone()
        .verify_active(peer_id, uuid, activation_epoch, activation_id, route_leases)
        .await
}

pub(crate) fn next_route_generation() -> u64 {
    REGISTRY.next_generation()
}

pub(crate) fn record_ready_ack() {
    REGISTRY.counters.ready_acks.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    REGISTRY.runtime_snapshot()
}

impl Registry {
    async fn begin_registration(
        self: Arc<Self>,
        peer_id: &str,
        uuid: &Bytes,
        public_key: &Bytes,
        activation_epoch: u64,
        activation_id: &Bytes,
        fast_identity_verified: bool,
    ) -> Result<ActivationClaim, ActivationError> {
        if peer_id.is_empty()
            || uuid.is_empty()
            || public_key.is_empty()
            || public_key.len() > PUBLIC_KEY_MAX_BYTES
            || (activation_id.is_empty() && activation_epoch != 0)
            || (!activation_id.is_empty()
                && (activation_id.len() != ACTIVATION_ID_BYTES
                    || activation_epoch == 0
                    || uuid.len() != 16))
        {
            self.counters
                .invalid_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Invalid);
        }
        let guard = self.peer_lock(peer_id).await?.lock_owned().await;
        let now = Instant::now();
        self.maintain(now).await;
        self.expire_route(peer_id, now).await;

        if activation_id.is_empty() {
            if self.v2_peers.read().await.contains(peer_id) {
                self.counters
                    .stale_rejections
                    .fetch_add(1, Ordering::Relaxed);
                return Err(ActivationError::Stale);
            }
            return Ok(ActivationClaim {
                registry: self.clone(),
                peer_id: peer_id.to_owned(),
                uuid: uuid.clone(),
                public_key: public_key.clone(),
                activation_epoch: 0,
                activation_id: Bytes::new(),
                route_lease: Bytes::new(),
                generation: self.next_generation(),
                previous_route: None,
                tracked: false,
                new_lease: false,
                fast_reregistration: false,
                capacity_reserved: false,
                _guard: guard,
            });
        }

        let mut previous_route = None;
        let mut existing_peer = false;
        if let Some(current) = self.leases.read().await.get(peer_id) {
            existing_peer = true;
            previous_route = current.route;
            if current.uuid != *uuid || activation_epoch < current.activation_epoch {
                self.counters
                    .stale_rejections
                    .fetch_add(1, Ordering::Relaxed);
                return Err(ActivationError::Stale);
            }
            if activation_epoch == current.activation_epoch {
                if current.activation_id != *activation_id
                    || current.public_key != *public_key
                    || current.retired
                {
                    self.counters
                        .stale_rejections
                        .fetch_add(1, Ordering::Relaxed);
                    return Err(ActivationError::Stale);
                }
                if current.route.is_some() {
                    if fast_identity_verified {
                        self.check_burst(peer_id, uuid, public_key, now).await?;
                    }
                    self.counters.leases_reused.fetch_add(1, Ordering::Relaxed);
                    return Ok(ActivationClaim {
                        registry: self.clone(),
                        peer_id: peer_id.to_owned(),
                        uuid: uuid.clone(),
                        public_key: public_key.clone(),
                        activation_epoch,
                        activation_id: activation_id.clone(),
                        route_lease: current.route_lease.clone(),
                        generation: current.generation,
                        previous_route: current.route,
                        tracked: true,
                        new_lease: false,
                        fast_reregistration: fast_identity_verified,
                        capacity_reserved: false,
                        _guard: guard,
                    });
                }
            }
        }

        if fast_identity_verified {
            self.check_burst(peer_id, uuid, public_key, now).await?;
        }
        self.remember_v2_peer(peer_id).await?;
        let capacity_reserved = if existing_peer {
            false
        } else {
            self.reserve_lease_capacity().await?
        };
        Ok(ActivationClaim {
            registry: self.clone(),
            peer_id: peer_id.to_owned(),
            uuid: uuid.clone(),
            public_key: public_key.clone(),
            activation_epoch,
            activation_id: activation_id.clone(),
            route_lease: sodiumoxide::randombytes::randombytes(ROUTE_LEASE_BYTES).into(),
            generation: self.next_generation(),
            previous_route,
            tracked: true,
            new_lease: true,
            fast_reregistration: fast_identity_verified,
            capacity_reserved,
            _guard: guard,
        })
    }

    async fn begin_renewal(
        self: Arc<Self>,
        peer_id: &str,
        activation_epoch: u64,
        activation_id: &Bytes,
        route_lease: &Bytes,
        route_generation: u64,
    ) -> Result<RenewalClaim, ActivationError> {
        if peer_id.is_empty()
            || (activation_id.is_empty()
                && (activation_epoch != 0 || !route_lease.is_empty() || route_generation != 0))
            || (!activation_id.is_empty()
                && (activation_id.len() != ACTIVATION_ID_BYTES
                    || activation_epoch == 0
                    || route_lease.len() != ROUTE_LEASE_BYTES
                    || route_generation == 0))
        {
            self.counters
                .invalid_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Invalid);
        }
        let guard = self.peer_lock(peer_id).await?.lock_owned().await;
        let now = Instant::now();
        self.expire_route(peer_id, now).await;
        let leases = self.leases.read().await;
        let (generation, previous_route, tracked) = match leases.get(peer_id) {
            None if activation_id.is_empty() => (0, None, false),
            Some(lease) if activation_id.is_empty() && (lease.route.is_none() || lease.retired) => {
                (0, None, false)
            }
            Some(lease)
                if !lease.retired
                    && lease.route.is_some()
                    && lease.activation_epoch == activation_epoch
                    && lease.activation_id == *activation_id
                    && lease.route_lease == *route_lease
                    && lease.generation == route_generation =>
            {
                (lease.generation, lease.route, true)
            }
            _ => {
                self.counters
                    .stale_rejections
                    .fetch_add(1, Ordering::Relaxed);
                return Err(ActivationError::Stale);
            }
        };
        drop(leases);
        Ok(RenewalClaim {
            registry: self,
            peer_id: peer_id.to_owned(),
            generation,
            previous_route,
            tracked,
            _guard: guard,
        })
    }

    async fn begin_deactivation(
        self: Arc<Self>,
        peer_id: &str,
        uuid: &Bytes,
        activation_epoch: u64,
        activation_id: &Bytes,
        route_lease: &Bytes,
        route_generation: u64,
    ) -> Result<DeactivationClaim, ActivationError> {
        if peer_id.is_empty()
            || uuid.len() != 16
            || activation_epoch == 0
            || activation_id.len() != ACTIVATION_ID_BYTES
            || route_lease.len() != ROUTE_LEASE_BYTES
            || route_generation == 0
        {
            self.counters
                .invalid_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Invalid);
        }
        let guard = self.peer_lock(peer_id).await?.lock_owned().await;
        self.expire_route(peer_id, Instant::now()).await;
        let leases = self.leases.read().await;
        let Some(lease) = leases.get(peer_id) else {
            self.counters
                .stale_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Stale);
        };
        if lease.uuid != *uuid
            || lease.activation_epoch != activation_epoch
            || lease.activation_id != *activation_id
            || lease.route_lease != *route_lease
            || lease.generation != route_generation
            || lease.retired
        {
            self.counters
                .stale_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Stale);
        }
        let Some(route) = lease.route else {
            self.counters
                .stale_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Stale);
        };
        let generation = lease.generation;
        drop(leases);
        let claim = DeactivationClaim {
            registry: self,
            peer_id: peer_id.to_owned(),
            activation_epoch,
            generation,
            route,
            _guard: guard,
        };
        Ok(claim)
    }

    async fn touch_route(self: Arc<Self>, peer_id: &str, route: RouteKind) -> bool {
        let Ok(lock) = self.peer_lock(peer_id).await else {
            return false;
        };
        let guard = lock.lock_owned().await;
        let mut leases = self.leases.write().await;
        let touched = leases.get_mut(peer_id).is_some_and(|lease| {
            if !lease.retired && lease.route == Some(route) {
                lease.updated_at = Instant::now();
                true
            } else {
                false
            }
        });
        drop(leases);
        drop(guard);
        touched
    }

    async fn disconnect_route(self: Arc<Self>, peer_id: &str, route: RouteKind) -> bool {
        let Ok(lock) = self.peer_lock(peer_id).await else {
            return false;
        };
        let guard = lock.lock_owned().await;
        let mut leases = self.leases.write().await;
        let disconnected = leases.get_mut(peer_id).is_some_and(|lease| {
            if !lease.retired && lease.route == Some(route) {
                lease.route = None;
                lease.route_lease = Bytes::new();
                lease.updated_at = Instant::now();
                true
            } else {
                false
            }
        });
        drop(leases);
        drop(guard);
        if disconnected {
            self.counters
                .disconnect_cleanups
                .fetch_add(1, Ordering::Relaxed);
        }
        disconnected
    }

    fn disconnect_websocket_routes_now(&self, routes: &[(String, u64, u64)]) -> usize {
        let Ok(mut leases) = self.leases.try_write() else {
            return 0;
        };
        let now = Instant::now();
        let mut disconnected = 0;
        for (peer_id, generation, connection_id) in routes {
            if leases.get_mut(peer_id).is_some_and(|lease| {
                if !lease.retired
                    && lease.route
                        == Some(RouteKind::WebSocket {
                            generation: *generation,
                            connection_id: *connection_id,
                        })
                {
                    lease.route = None;
                    lease.route_lease = Bytes::new();
                    lease.updated_at = now;
                    true
                } else {
                    false
                }
            }) {
                disconnected += 1;
            }
        }
        self.counters
            .disconnect_cleanups
            .fetch_add(disconnected as u64, Ordering::Relaxed);
        disconnected
    }

    async fn verify_active(
        self: Arc<Self>,
        peer_id: &str,
        uuid: &Bytes,
        activation_epoch: u64,
        activation_id: &Bytes,
        route_leases: &[Bytes],
    ) -> bool {
        if peer_id.is_empty()
            || uuid.len() != 16
            || activation_epoch == 0
            || activation_id.len() != ACTIVATION_ID_BYTES
            || route_leases.is_empty()
            || route_leases.len() > 16
            || route_leases
                .iter()
                .any(|route_lease| route_lease.len() != ROUTE_LEASE_BYTES)
        {
            return false;
        }
        let Ok(lock) = self.peer_lock(peer_id).await else {
            return false;
        };
        let guard = lock.lock_owned().await;
        self.expire_route(peer_id, Instant::now()).await;
        let leases = self.leases.read().await;
        let verified = leases.get(peer_id).is_some_and(|lease| {
            !lease.retired
                && lease.route.is_some()
                && lease.uuid == *uuid
                && lease.activation_epoch == activation_epoch
                && lease.activation_id == *activation_id
                && route_leases
                    .iter()
                    .any(|route_lease| *route_lease == lease.route_lease)
        });
        drop(leases);
        drop(guard);
        verified
    }

    fn next_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::SeqCst)
    }

    async fn peer_lock(&self, peer_id: &str) -> Result<Arc<Mutex<()>>, ActivationError> {
        let mut locks = self.peer_locks.lock().await;
        if let Some(lock) = locks.get(peer_id) {
            return Ok(lock.clone());
        }
        if locks.len() >= MAX_PEER_LOCKS {
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        if locks.len() >= MAX_PEER_LOCKS {
            self.counters
                .capacity_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Capacity);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(peer_id.to_owned(), lock.clone());
        Ok(lock)
    }

    async fn reserve_lease_capacity(&self) -> Result<bool, ActivationError> {
        let active = self.leases.read().await.len();
        let pending = self.pending_new_leases.fetch_add(1, Ordering::AcqRel);
        if active.saturating_add(pending) >= MAX_LEASES {
            self.pending_new_leases.fetch_sub(1, Ordering::AcqRel);
            self.counters
                .capacity_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Capacity);
        }
        Ok(true)
    }

    async fn remember_v2_peer(&self, peer_id: &str) -> Result<(), ActivationError> {
        let mut peers = self.v2_peers.write().await;
        if peers.contains(peer_id) {
            return Ok(());
        }
        if peers.len() >= MAX_V2_PEERS {
            self.counters
                .capacity_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Capacity);
        }
        peers.insert(peer_id.to_owned());
        Ok(())
    }

    async fn check_burst(
        &self,
        peer_id: &str,
        uuid: &Bytes,
        public_key: &Bytes,
        now: Instant,
    ) -> Result<(), ActivationError> {
        let public_key_sha256: [u8; 32] = Sha256::digest(public_key).into();
        let key = IdentityKey {
            peer_id: peer_id.to_owned(),
            uuid: uuid.to_vec(),
            public_key_sha256,
        };
        let mut bursts = self.bursts.lock().await;
        if self.maintenance_calls.load(Ordering::Relaxed) % MAINTENANCE_INTERVAL == 0
            || (!bursts.contains_key(&key) && bursts.len() >= MAX_BURST_IDENTITIES)
        {
            bursts.retain(|_, entries| {
                while entries
                    .front()
                    .is_some_and(|entry| now.duration_since(*entry) >= BURST_WINDOW)
                {
                    entries.pop_front();
                }
                !entries.is_empty()
            });
        }
        if !bursts.contains_key(&key) && bursts.len() >= MAX_BURST_IDENTITIES {
            self.counters
                .capacity_rejections
                .fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::Capacity);
        }
        let entries = bursts.entry(key).or_default();
        while entries
            .front()
            .is_some_and(|entry| now.duration_since(*entry) >= BURST_WINDOW)
        {
            entries.pop_front();
        }
        if entries.len() >= BURST_LIMIT {
            self.counters.rate_limited.fetch_add(1, Ordering::Relaxed);
            return Err(ActivationError::RateLimited);
        }
        entries.push_back(now);
        Ok(())
    }

    async fn expire_route(&self, peer_id: &str, now: Instant) {
        let mut leases = self.leases.write().await;
        if let Some(lease) = leases.get_mut(peer_id).filter(|lease| {
            lease.route.is_some() && now.duration_since(lease.updated_at) >= LEASE_TTL
        }) {
            lease.route = None;
            lease.route_lease = Bytes::new();
            lease.updated_at = now;
            self.counters
                .ttl_expirations
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn maintain(&self, now: Instant) {
        let call = self.maintenance_calls.fetch_add(1, Ordering::Relaxed);
        if call % MAINTENANCE_INTERVAL != 0 {
            return;
        }
        let mut leases = self.leases.write().await;
        leases.retain(|_, lease| {
            if lease.route.is_some() && now.duration_since(lease.updated_at) >= LEASE_TTL {
                lease.route = None;
                lease.route_lease = Bytes::new();
                lease.updated_at = now;
                self.counters
                    .ttl_expirations
                    .fetch_add(1, Ordering::Relaxed);
            }
            lease.route.is_some() || now.duration_since(lease.updated_at) < RECORD_RETENTION
        });
    }

    fn runtime_snapshot(&self) -> RuntimeSnapshot {
        let active_leases = self
            .leases
            .try_read()
            .map(|leases| {
                leases
                    .values()
                    .filter(|lease| !lease.retired && lease.route.is_some())
                    .count()
            })
            .unwrap_or_default();
        RuntimeSnapshot {
            protocol_version: PROTOCOL_VERSION,
            active_leases,
            last_route_generation: self
                .next_generation
                .load(Ordering::Relaxed)
                .saturating_sub(1),
            leases_issued: self.counters.leases_issued.load(Ordering::Relaxed),
            leases_reused: self.counters.leases_reused.load(Ordering::Relaxed),
            ready_acks: self.counters.ready_acks.load(Ordering::Relaxed),
            fast_reregistrations: self.counters.fast_reregistrations.load(Ordering::Relaxed),
            renewals: self.counters.renewals.load(Ordering::Relaxed),
            route_replacements: self.counters.route_replacements.load(Ordering::Relaxed),
            deactivations: self.counters.deactivations.load(Ordering::Relaxed),
            disconnect_cleanups: self.counters.disconnect_cleanups.load(Ordering::Relaxed),
            ttl_expirations: self.counters.ttl_expirations.load(Ordering::Relaxed),
            invalid_requests: self.counters.invalid_requests.load(Ordering::Relaxed),
            stale_rejections: self.counters.stale_rejections.load(Ordering::Relaxed),
            rate_limited: self.counters.rate_limited.load(Ordering::Relaxed),
            capacity_rejections: self.counters.capacity_rejections.load(Ordering::Relaxed),
            lease_ttl_seconds: LEASE_TTL_SECONDS,
            burst_window_seconds: BURST_WINDOW_SECONDS,
            burst_limit: BURST_LIMIT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;

    fn registry() -> Arc<Registry> {
        Arc::new(Registry::default())
    }

    fn identity(byte: u8) -> (Bytes, Bytes) {
        (vec![byte; 16].into(), vec![byte.wrapping_add(1); 32].into())
    }

    async fn register(
        registry: &Arc<Registry>,
        peer: &str,
        uuid: &Bytes,
        pk: &Bytes,
        epoch: u64,
        activation_byte: u8,
        route: RouteKind,
        verified: bool,
    ) -> (Bytes, u64) {
        let activation_id: Bytes = vec![activation_byte; ACTIVATION_ID_BYTES].into();
        let claim = registry
            .clone()
            .begin_registration(peer, uuid, pk, epoch, &activation_id, verified)
            .await
            .unwrap();
        let lease = claim.route_lease().clone();
        let generation = claim.generation();
        claim.commit(route).await;
        (lease, generation)
    }

    #[tokio::test]
    async fn stale_renewal_and_deactivation_cannot_replace_current_lease() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(1);
        let first_id: Bytes = vec![2; ACTIVATION_ID_BYTES].into();
        let (old_lease, old_generation) = register(
            &registry,
            &peer,
            &uuid,
            &pk,
            1,
            2,
            RouteKind::Native("127.0.0.1:1000".parse().unwrap()),
            false,
        )
        .await;
        let second_id: Bytes = vec![3; ACTIVATION_ID_BYTES].into();
        let (current_lease, current_generation) = register(
            &registry,
            &peer,
            &uuid,
            &pk,
            2,
            3,
            RouteKind::Native("127.0.0.1:2000".parse().unwrap()),
            true,
        )
        .await;

        assert!(registry
            .clone()
            .begin_renewal(&peer, 1, &first_id, &old_lease, old_generation)
            .await
            .is_err());
        assert!(registry
            .clone()
            .begin_deactivation(&peer, &uuid, 1, &first_id, &old_lease, old_generation,)
            .await
            .is_err());
        assert!(registry
            .clone()
            .begin_renewal(&peer, 2, &second_id, &current_lease, current_generation,)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_to_b_to_a_and_out_of_order_packets_keep_the_newest_activation() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(4);
        let (lease_a1, generation_a1) = register(
            &registry,
            &peer,
            &uuid,
            &pk,
            1,
            10,
            RouteKind::Native("127.0.0.1:3001".parse().unwrap()),
            false,
        )
        .await;
        let (lease_b, generation_b) = register(
            &registry,
            &peer,
            &uuid,
            &pk,
            2,
            11,
            RouteKind::WebSocket {
                generation: 2,
                connection_id: 20,
            },
            true,
        )
        .await;
        let (lease_a2, generation_a2) = register(
            &registry,
            &peer,
            &uuid,
            &pk,
            3,
            12,
            RouteKind::Native("127.0.0.1:3002".parse().unwrap()),
            true,
        )
        .await;
        assert!(generation_a1 < generation_b && generation_b < generation_a2);

        for (epoch, id, lease, generation) in [
            (
                1,
                vec![10; ACTIVATION_ID_BYTES].into(),
                lease_a1,
                generation_a1,
            ),
            (
                2,
                vec![11; ACTIVATION_ID_BYTES].into(),
                lease_b,
                generation_b,
            ),
        ] {
            assert!(registry
                .clone()
                .begin_deactivation(&peer, &uuid, epoch, &id, &lease, generation)
                .await
                .is_err());
        }
        let current_id: Bytes = vec![12; ACTIVATION_ID_BYTES].into();
        assert!(
            registry
                .clone()
                .verify_active(
                    &peer,
                    &uuid,
                    3,
                    &current_id,
                    std::slice::from_ref(&lease_a2),
                )
                .await
        );
    }

    #[tokio::test]
    async fn legacy_registration_cannot_override_an_active_leased_route() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(6);
        register(
            &registry,
            &peer,
            &uuid,
            &pk,
            1,
            7,
            RouteKind::Native("127.0.0.1:4000".parse().unwrap()),
            false,
        )
        .await;
        assert!(matches!(
            registry
                .clone()
                .begin_registration(&peer, &uuid, &pk, 0, &Bytes::new(), false)
                .await,
            Err(ActivationError::Stale)
        ));
    }

    #[tokio::test]
    async fn deactivated_v2_peer_cannot_downgrade_after_lease_retention_cleanup() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(7);
        let activation_id: Bytes = vec![8; ACTIVATION_ID_BYTES].into();
        let route = RouteKind::Native("127.0.0.1:4100".parse().unwrap());
        let (lease, generation) = register(&registry, &peer, &uuid, &pk, 1, 8, route, false).await;
        let claim = registry
            .clone()
            .begin_deactivation(&peer, &uuid, 1, &activation_id, &lease, generation)
            .await
            .unwrap();
        assert!(claim.commit(true).await);
        registry.leases.write().await.remove(&peer);

        assert!(matches!(
            registry
                .clone()
                .begin_registration(&peer, &uuid, &pk, 0, &Bytes::new(), false)
                .await,
            Err(ActivationError::Stale)
        ));
    }

    #[tokio::test]
    async fn only_the_exact_generation_and_route_lease_can_deactivate() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(8);
        let activation_id: Bytes = vec![9; ACTIVATION_ID_BYTES].into();
        let route = RouteKind::Native("127.0.0.1:5000".parse().unwrap());
        let (lease, generation) = register(&registry, &peer, &uuid, &pk, 9, 9, route, false).await;
        assert!(registry
            .clone()
            .begin_deactivation(&peer, &uuid, 9, &activation_id, &lease, generation + 1,)
            .await
            .is_err());
        assert!(registry
            .clone()
            .begin_deactivation(
                &peer,
                &uuid,
                9,
                &activation_id,
                &vec![1; ROUTE_LEASE_BYTES].into(),
                generation,
            )
            .await
            .is_err());
        let claim = registry
            .clone()
            .begin_deactivation(&peer, &uuid, 9, &activation_id, &lease, generation)
            .await
            .unwrap();
        assert_eq!(claim.route(), route);
        assert!(claim.commit(true).await);
        assert!(registry
            .clone()
            .begin_registration(&peer, &uuid, &pk, 9, &activation_id, true)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn disconnect_cleanup_allows_same_activation_to_recover_with_a_new_lease() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(10);
        let activation_id: Bytes = vec![11; ACTIVATION_ID_BYTES].into();
        let route = RouteKind::WebSocket {
            generation: 1,
            connection_id: 10,
        };
        let (old_lease, old_generation) =
            register(&registry, &peer, &uuid, &pk, 1, 11, route, false).await;
        assert!(registry.clone().disconnect_route(&peer, route).await);
        let claim = registry
            .clone()
            .begin_registration(&peer, &uuid, &pk, 1, &activation_id, true)
            .await
            .unwrap();
        assert_ne!(claim.route_lease(), &old_lease);
        assert!(claim.generation() > old_generation);
    }

    #[tokio::test]
    async fn verified_fast_reregistration_has_an_independent_bounded_burst() {
        let registry = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(12);
        let route = RouteKind::Native("127.0.0.1:6000".parse().unwrap());
        let (current_lease, current_generation) =
            register(&registry, &peer, &uuid, &pk, 1, 13, route, false).await;
        let current_id: Bytes = vec![13; ACTIVATION_ID_BYTES].into();
        let retry = registry
            .clone()
            .begin_registration(&peer, &uuid, &pk, 1, &current_id, true)
            .await
            .unwrap();
        assert_eq!(retry.route_lease(), &current_lease);
        assert_eq!(retry.generation(), current_generation);
        retry.commit(route).await;

        for offset in 0..BURST_LIMIT - 1 {
            let activation_id: Bytes = vec![20 + offset as u8; ACTIVATION_ID_BYTES].into();
            registry
                .clone()
                .begin_registration(&peer, &uuid, &pk, 2 + offset as u64, &activation_id, true)
                .await
                .unwrap()
                .commit(RouteKind::Native(
                    format!("127.0.0.1:{}", 6100 + offset).parse().unwrap(),
                ))
                .await;
        }
        let denied_id: Bytes = vec![99; ACTIVATION_ID_BYTES].into();
        assert!(matches!(
            registry
                .clone()
                .begin_registration(&peer, &uuid, &pk, 100, &denied_id, true,)
                .await,
            Err(ActivationError::RateLimited)
        ));
        assert_eq!(registry.runtime_snapshot().leases_reused, 1);
        assert_eq!(registry.runtime_snapshot().rate_limited, 1);
    }

    #[tokio::test]
    async fn two_nodes_issue_independent_leases_and_deactivate_independently() {
        let node_a = registry();
        let node_b = registry();
        let peer = format!("peer-{}", uuid::Uuid::new_v4());
        let (uuid, pk) = identity(14);
        let activation_id: Bytes = vec![15; ACTIVATION_ID_BYTES].into();
        let (lease_a, generation_a) = register(
            &node_a,
            &peer,
            &uuid,
            &pk,
            1,
            15,
            RouteKind::Native("127.0.0.1:7001".parse().unwrap()),
            false,
        )
        .await;
        let (lease_b, generation_b) = register(
            &node_b,
            &peer,
            &uuid,
            &pk,
            1,
            15,
            RouteKind::Native("127.0.0.1:7002".parse().unwrap()),
            false,
        )
        .await;
        assert_ne!(lease_a, lease_b);
        let claim = node_a
            .clone()
            .begin_deactivation(&peer, &uuid, 1, &activation_id, &lease_a, generation_a)
            .await
            .unwrap();
        assert!(claim.commit(true).await);
        assert!(
            node_b
                .clone()
                .verify_active(
                    &peer,
                    &uuid,
                    1,
                    &activation_id,
                    std::slice::from_ref(&lease_b),
                )
                .await
        );
        assert!(node_b
            .clone()
            .begin_deactivation(&peer, &uuid, 1, &activation_id, &lease_a, generation_b,)
            .await
            .is_err());
    }
}
