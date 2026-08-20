use super::session::{SendError, WsWriteTransport};
use hbb_common::{log, tokio::sync::RwLock};
use once_cell::sync::Lazy;
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

static CLOCK: Lazy<Instant> = Lazy::new(Instant::now);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static SESSIONS: Lazy<RwLock<HashMap<String, Session>>> = Lazy::new(|| RwLock::new(HashMap::new()));
static CONNECTIONS: Lazy<RwLock<HashMap<SocketAddr, ConnectionContext>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static REGISTRATIONS: Lazy<RwLock<HashMap<IpAddr, VecDeque<Instant>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
const MAX_REGISTRATION_IPS: usize = 65_536;
const REGISTRATION_SWEEP_INTERVAL: usize = 256;
static REGISTRATION_CALLS: AtomicUsize = AtomicUsize::new(0);

static REGISTERED: AtomicUsize = AtomicUsize::new(0);
static REPLACED: AtomicUsize = AtomicUsize::new(0);
static EVICTED: AtomicUsize = AtomicUsize::new(0);
static TIMED_OUT: AtomicUsize = AtomicUsize::new(0);
static SLOW_CONSUMERS: AtomicUsize = AtomicUsize::new(0);

struct Session {
    generation: u64,
    writer: WsWriteTransport,
    effective_ip: IpAddr,
    route_addr: SocketAddr,
    connected_at_millis: u64,
    last_seen_millis: Arc<AtomicU64>,
}

#[derive(Clone)]
struct ConnectionContext {
    connection_id: u64,
    effective_addr: SocketAddr,
    writer: WsWriteTransport,
}

#[derive(Clone)]
pub(crate) struct SessionToken {
    pub(crate) peer_id: String,
    pub(crate) generation: u64,
    pub(crate) last_seen_millis: Arc<AtomicU64>,
}

impl SessionToken {
    pub(crate) fn touch(&self) {
        self.last_seen_millis.store(now_millis(), Ordering::Release);
    }

    pub(crate) fn idle_for(&self) -> Duration {
        Duration::from_millis(
            now_millis().saturating_sub(self.last_seen_millis.load(Ordering::Acquire)),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionRoute {
    pub(crate) generation: u64,
    pub(crate) route_addr: SocketAddr,
    pub(crate) effective_ip: IpAddr,
    pub(crate) idle_millis: u64,
}

#[derive(Default)]
pub(crate) struct RoutingStatus {
    pub(crate) sessions: usize,
    pub(crate) draining: usize,
    pub(crate) registered: usize,
    pub(crate) replaced: usize,
    pub(crate) evicted: usize,
    pub(crate) timed_out: usize,
    pub(crate) slow_consumers: usize,
}

pub(crate) async fn register_connection(
    route_addr: SocketAddr,
    effective_addr: SocketAddr,
    connection_id: u64,
    writer: WsWriteTransport,
) {
    CONNECTIONS.write().await.insert(
        route_addr,
        ConnectionContext {
            connection_id,
            effective_addr,
            writer,
        },
    );
}

pub(crate) async fn remove_connection(route_addr: SocketAddr, connection_id: u64) {
    let mut connections = CONNECTIONS.write().await;
    if connections
        .get(&route_addr)
        .map(|entry| entry.connection_id == connection_id)
        .unwrap_or(false)
    {
        connections.remove(&route_addr);
    }
}

pub(crate) async fn connection_effective(route_addr: SocketAddr) -> Option<SocketAddr> {
    CONNECTIONS
        .read()
        .await
        .get(&route_addr)
        .map(|entry| entry.effective_addr)
}

pub(crate) async fn is_websocket_route(route_addr: SocketAddr) -> bool {
    CONNECTIONS.read().await.contains_key(&route_addr)
}

pub(crate) async fn allow_registration(effective_ip: IpAddr, limit: usize) -> bool {
    let now = Instant::now();
    let cutoff = Duration::from_secs(60);
    let mut registrations = REGISTRATIONS.write().await;
    let calls = REGISTRATION_CALLS.fetch_add(1, Ordering::Relaxed);
    if calls % REGISTRATION_SWEEP_INTERVAL == 0 {
        registrations.retain(|_, entries| {
            while entries
                .front()
                .map(|instant| now.duration_since(*instant) >= cutoff)
                .unwrap_or(false)
            {
                entries.pop_front();
            }
            !entries.is_empty()
        });
    }
    if !registrations.contains_key(&effective_ip) && registrations.len() >= MAX_REGISTRATION_IPS {
        return false;
    }
    let entries = registrations.entry(effective_ip).or_default();
    while entries
        .front()
        .map(|instant| now.duration_since(*instant) >= cutoff)
        .unwrap_or(false)
    {
        entries.pop_front();
    }
    if entries.len() >= limit {
        return false;
    }
    entries.push_back(now);
    true
}

pub(crate) async fn capacity_available(
    peer_id: &str,
    effective_ip: IpAddr,
    max_sessions: usize,
    max_sessions_per_ip: usize,
) -> bool {
    let sessions = SESSIONS.read().await;
    let replacing = sessions.contains_key(peer_id);
    if !replacing && sessions.len() >= max_sessions {
        return false;
    }
    sessions
        .iter()
        .filter(|(id, session)| id.as_str() != peer_id && session.effective_ip == effective_ip)
        .count()
        < max_sessions_per_ip
}

pub(crate) async fn bind(
    peer_id: String,
    writer: WsWriteTransport,
    effective_ip: IpAddr,
    route_addr: SocketAddr,
    max_sessions: usize,
    max_sessions_per_ip: usize,
) -> Result<SessionToken, String> {
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    let last_seen_millis = Arc::new(AtomicU64::new(now_millis()));
    let new = Session {
        generation,
        writer,
        effective_ip,
        route_addr,
        connected_at_millis: now_millis(),
        last_seen_millis: last_seen_millis.clone(),
    };
    let mut sessions = SESSIONS.write().await;
    let replacing = sessions.contains_key(&peer_id);
    if !replacing && sessions.len() >= max_sessions {
        return Err("global WebSocket session limit reached".to_owned());
    }
    let same_ip = sessions
        .iter()
        .filter(|(id, session)| id.as_str() != peer_id && session.effective_ip == effective_ip)
        .count();
    if same_ip >= max_sessions_per_ip {
        return Err("per-effective-IP WebSocket session limit reached".to_owned());
    }
    if let Some(previous) = sessions.insert(peer_id.clone(), new) {
        previous.writer.abort();
        REPLACED.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "WebSocket signal route replaced: peer={}, generation={} -> {}",
            redacted_peer(&peer_id),
            previous.generation,
            generation
        );
    } else {
        REGISTERED.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "WebSocket signal route registered: peer={}, generation={}",
            redacted_peer(&peer_id),
            generation
        );
    }
    Ok(SessionToken {
        peer_id,
        generation,
        last_seen_millis,
    })
}

pub(crate) async fn route(peer_id: &str) -> Option<SessionRoute> {
    let sessions = SESSIONS.read().await;
    let session = sessions.get(peer_id)?;
    Some(SessionRoute {
        generation: session.generation,
        route_addr: session.route_addr,
        effective_ip: session.effective_ip,
        idle_millis: now_millis().saturating_sub(session.last_seen_millis.load(Ordering::Acquire)),
    })
}

pub(crate) async fn try_send(peer_id: &str, bytes: Vec<u8>) -> bool {
    let candidate = {
        let sessions = SESSIONS.read().await;
        sessions
            .get(peer_id)
            .map(|session| (session.generation, session.writer.clone()))
    };
    let Some((generation, writer)) = candidate else {
        return false;
    };
    match writer.send_binary(bytes) {
        Ok(()) => true,
        Err(SendError::Full) => {
            SLOW_CONSUMERS.fetch_add(1, Ordering::Relaxed);
            remove_if_current(peer_id, generation, "slow consumer").await;
            false
        }
        Err(SendError::Closed) => {
            remove_if_current(peer_id, generation, "writer closed").await;
            false
        }
    }
}

pub(crate) async fn remove_if_current(peer_id: &str, generation: u64, reason: &str) -> bool {
    let removed = {
        let mut sessions = SESSIONS.write().await;
        if sessions
            .get(peer_id)
            .map(|session| session.generation == generation)
            .unwrap_or(false)
        {
            sessions.remove(peer_id)
        } else {
            None
        }
    };
    if let Some(session) = removed {
        session.writer.abort();
        if reason == "idle timeout" {
            TIMED_OUT.fetch_add(1, Ordering::Relaxed);
        }
        log::info!(
            "WebSocket signal route removed: peer={}, generation={}, reason={}",
            redacted_peer(peer_id),
            generation,
            reason
        );
        true
    } else {
        false
    }
}

pub(crate) async fn native_registration(peer_id: &str) -> bool {
    // Native RegisterPeer proves network reachability, not ownership of the
    // generation-bound WSS identity.  Preserve the stronger route until its
    // own reader closes, its writer fails, or its absolute idle deadline
    // removes it.  A future immediate transition must carry an explicit
    // identity binding rather than treating a shared public address as proof.
    let _ = peer_id;
    false
}

pub(crate) async fn drain_all() -> usize {
    let sessions = {
        let mut guard = SESSIONS.write().await;
        std::mem::take(&mut *guard)
    };
    let count = sessions.len();
    for session in sessions.into_values() {
        session.writer.abort();
    }
    count
}

pub(crate) fn drain_all_now() -> Result<(usize, usize), String> {
    let mut sessions = SESSIONS
        .try_write()
        .map_err(|_| "WebSocket session registry is busy; retry activation".to_owned())?;
    let mut connections = CONNECTIONS
        .try_write()
        .map_err(|_| "WebSocket connection registry is busy; retry activation".to_owned())?;
    let drained_sessions = std::mem::take(&mut *sessions);
    let drained_connections = std::mem::take(&mut *connections);
    let session_count = drained_sessions.len();
    let connection_count = drained_connections.len();
    for session in drained_sessions.into_values() {
        session.writer.abort();
    }
    for connection in drained_connections.into_values() {
        connection.writer.abort();
    }
    Ok((session_count, connection_count))
}

pub(crate) async fn status() -> RoutingStatus {
    let sessions = SESSIONS.read().await;
    let _oldest_age = sessions
        .values()
        .map(|session| now_millis().saturating_sub(session.connected_at_millis))
        .max()
        .unwrap_or(0);
    RoutingStatus {
        sessions: sessions.len(),
        draining: sessions
            .values()
            .filter(|session| session.writer.is_closed())
            .count(),
        registered: REGISTERED.load(Ordering::Relaxed),
        replaced: REPLACED.load(Ordering::Relaxed),
        evicted: EVICTED.load(Ordering::Relaxed),
        timed_out: TIMED_OUT.load(Ordering::Relaxed),
        slow_consumers: SLOW_CONSUMERS.load(Ordering::Relaxed),
    }
}

fn now_millis() -> u64 {
    CLOCK.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn redacted_peer(peer_id: &str) -> String {
    let suffix: String = peer_id
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("***{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ROUTING_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn old_generation_cannot_remove_replacement() {
        let _guard = ROUTING_TEST_LOCK.lock().unwrap();
        hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let peer = format!("generation-test-{}", std::process::id());
                let (first, _first_rx) = WsWriteTransport::channel(1, 4);
                let first = bind(
                    peer.clone(),
                    first,
                    "192.0.2.1".parse().unwrap(),
                    "127.0.0.1:10001".parse().unwrap(),
                    10,
                    10,
                )
                .await
                .unwrap();
                let (second, _second_rx) = WsWriteTransport::channel(2, 4);
                let second = bind(
                    peer.clone(),
                    second,
                    "192.0.2.1".parse().unwrap(),
                    "127.0.0.1:10002".parse().unwrap(),
                    10,
                    10,
                )
                .await
                .unwrap();
                assert!(!remove_if_current(&peer, first.generation, "old reader").await);
                assert_eq!(route(&peer).await.unwrap().generation, second.generation);
                assert!(remove_if_current(&peer, second.generation, "test cleanup").await);

                let (third, _third_rx) = WsWriteTransport::channel(3, 4);
                bind(
                    peer.clone(),
                    third,
                    "192.0.2.1".parse().unwrap(),
                    "127.0.0.1:10003".parse().unwrap(),
                    10,
                    10,
                )
                .await
                .unwrap();
                assert!(!native_registration(&peer).await);
                let current = route(&peer).await.unwrap();
                assert!(remove_if_current(&peer, current.generation, "test cleanup").await);
                assert!(route(&peer).await.is_none());
            });
    }

    #[test]
    fn registry_sustains_one_thousand_idle_sessions_and_reconnects() {
        let _guard = ROUTING_TEST_LOCK.lock().unwrap();
        hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                drain_all().await;
                let mut receivers = Vec::with_capacity(1_100);
                for index in 0..1_000_u16 {
                    let peer = format!("idle-load-{index:04}");
                    let (writer, receiver) = WsWriteTransport::channel(u64::from(index) + 1, 4);
                    bind(
                        peer,
                        writer,
                        "192.0.2.10".parse().unwrap(),
                        SocketAddr::from(([127, 0, 0, 1], 20_000 + index)),
                        1_000,
                        1_000,
                    )
                    .await
                    .unwrap();
                    receivers.push(receiver);
                }

                let snapshot = status().await;
                assert_eq!(snapshot.sessions, 1_000);
                assert_eq!(snapshot.draining, 0);
                assert!(
                    !capacity_available(
                        "idle-load-overflow",
                        "192.0.2.11".parse().unwrap(),
                        1_000,
                        1_000,
                    )
                    .await
                );

                // Replace a canary subset as concurrent clients would during
                // reconnect storms. Old generations are aborted atomically and
                // the registry remains at its configured bound.
                for index in 0..100_u16 {
                    let peer = format!("idle-load-{index:04}");
                    let (writer, receiver) =
                        WsWriteTransport::channel(u64::from(index) + 10_001, 4);
                    let token = bind(
                        peer.clone(),
                        writer,
                        "192.0.2.10".parse().unwrap(),
                        SocketAddr::from(([127, 0, 0, 1], 30_000 + index)),
                        1_000,
                        1_000,
                    )
                    .await
                    .unwrap();
                    assert_eq!(route(&peer).await.unwrap().generation, token.generation);
                    receivers.push(receiver);
                }
                assert_eq!(status().await.sessions, 1_000);
                assert_eq!(drain_all().await, 1_000);
                assert_eq!(status().await.sessions, 0);
                drop(receivers);
            });
    }
}
