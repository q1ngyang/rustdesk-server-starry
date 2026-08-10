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

#[derive(Clone, Copy)]
struct ConnectionContext {
    connection_id: u64,
    effective_addr: SocketAddr,
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
) {
    CONNECTIONS.write().await.insert(
        route_addr,
        ConnectionContext {
            connection_id,
            effective_addr,
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
    let removed = SESSIONS.write().await.remove(peer_id);
    if let Some(session) = removed {
        session.writer.abort();
        EVICTED.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "WebSocket signal route evicted by native registration: peer={}, generation={}",
            redacted_peer(peer_id),
            session.generation
        );
        true
    } else {
        false
    }
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

    #[test]
    fn old_generation_cannot_remove_replacement() {
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
                assert!(native_registration(&peer).await);
                assert!(route(&peer).await.is_none());
            });
    }
}
