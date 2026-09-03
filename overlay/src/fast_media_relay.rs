use hbb_common::{
    log,
    protobuf::Message as _,
    rendezvous_proto::FastRelayAuthorization,
    tokio::{
        self,
        net::UdpSocket,
        time::{interval, sleep, Duration},
    },
};
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::{auth, sign};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const TELEMETRY_SCHEMA_VERSION: u32 = 2;
const AKR_HEADER_BYTES: usize = 32;
const HELLO_COOKIE_BYTES: usize = 56;
const BIND_PREFIX_BYTES: usize = 58;
const MAX_AUTHORIZATION_BYTES: usize = 4_096;
const MIN_RELAY_DATAGRAM: u32 = 608;
const MAX_RELAY_DATAGRAM: u32 = 1_400;
const MIN_AKF1_BYTES: usize = 22 + 16 + 51;
const MAX_GRANT_TTL_SECONDS: u64 = 300;
const DEFAULT_HALF_BIND_TTL_SECONDS: u64 = 10;
const DEFAULT_IDLE_TTL_SECONDS: u64 = 30;
const DEFAULT_MAX_ALLOCATIONS: usize = 10_000;
const DEFAULT_PER_IP_PACKETS_PER_SECOND: u64 = 20_000;
const DEFAULT_GLOBAL_PACKETS_PER_SECOND: u64 = 250_000;
const DEFAULT_PER_IP_BYTES_PER_SECOND: u64 = 32 * 1024 * 1024;
const DEFAULT_GLOBAL_BYTES_PER_SECOND: u64 = 512 * 1024 * 1024;
const MAX_IP_BUCKETS: usize = 4_096;
const CLEANUP_PER_TICK: usize = 64;
const COOKIE_EPOCH_SECONDS: u64 = 10;
const REBINDS_PER_MINUTE: u32 = 12;
const ROLE_CONTROLLER: u8 = 1;
const ROLE_TARGET: u8 = 2;

static ENABLED: AtomicBool = AtomicBool::new(false);
static HEALTHY: AtomicBool = AtomicBool::new(false);
static UDP_PORT: AtomicU16 = AtomicU16::new(0);
static ACTIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_STREAMS: AtomicUsize = AtomicUsize::new(0);
static HELLO_ACCEPTED: AtomicU64 = AtomicU64::new(0);
static COOKIE_REJECTED: AtomicU64 = AtomicU64::new(0);
static BIND_SUCCEEDED: AtomicU64 = AtomicU64::new(0);
static BIND_REJECTED: AtomicU64 = AtomicU64::new(0);
static GRANT_REJECTED: AtomicU64 = AtomicU64::new(0);
static ROLE_MISMATCH: AtomicU64 = AtomicU64::new(0);
static SESSION_MISMATCH: AtomicU64 = AtomicU64::new(0);
static ALLOCATION_MISMATCH: AtomicU64 = AtomicU64::new(0);
static REBINDS: AtomicU64 = AtomicU64::new(0);
static FORWARDED_PACKETS: AtomicU64 = AtomicU64::new(0);
static FORWARDED_BYTES: AtomicU64 = AtomicU64::new(0);
static DROPPED_PACKETS: AtomicU64 = AtomicU64::new(0);
static RATE_LIMITED: AtomicU64 = AtomicU64::new(0);
static REPLAY_REJECTED: AtomicU64 = AtomicU64::new(0);
static EXPIRED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LISTENER_FAILURES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, serde_derive::Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) protocol_version: u32,
    pub(crate) enabled: bool,
    pub(crate) healthy: bool,
    pub(crate) udp_port: Option<u16>,
    pub(crate) active_allocations: usize,
    pub(crate) active_streams: usize,
    pub(crate) hello_accepted: u64,
    pub(crate) cookie_rejected: u64,
    pub(crate) bind_succeeded: u64,
    pub(crate) bind_rejected: u64,
    pub(crate) grant_rejected: u64,
    pub(crate) role_mismatch: u64,
    pub(crate) session_mismatch: u64,
    pub(crate) allocation_mismatch: u64,
    pub(crate) rebinds: u64,
    pub(crate) forwarded_packets: u64,
    pub(crate) forwarded_bytes: u64,
    pub(crate) dropped_packets: u64,
    pub(crate) rate_limited: u64,
    pub(crate) replay_rejected: u64,
    pub(crate) expired_allocations: u64,
    pub(crate) listener_failures: u64,
}

pub(crate) fn runtime_snapshot() -> RuntimeSnapshot {
    let port = UDP_PORT.load(Ordering::Relaxed);
    RuntimeSnapshot {
        protocol_version: PROTOCOL_VERSION,
        enabled: ENABLED.load(Ordering::Relaxed),
        healthy: HEALTHY.load(Ordering::Relaxed),
        udp_port: (port > 0).then_some(port),
        active_allocations: ACTIVE_ALLOCATIONS.load(Ordering::Relaxed),
        active_streams: ACTIVE_STREAMS.load(Ordering::Relaxed),
        hello_accepted: HELLO_ACCEPTED.load(Ordering::Relaxed),
        cookie_rejected: COOKIE_REJECTED.load(Ordering::Relaxed),
        bind_succeeded: BIND_SUCCEEDED.load(Ordering::Relaxed),
        bind_rejected: BIND_REJECTED.load(Ordering::Relaxed),
        grant_rejected: GRANT_REJECTED.load(Ordering::Relaxed),
        role_mismatch: ROLE_MISMATCH.load(Ordering::Relaxed),
        session_mismatch: SESSION_MISMATCH.load(Ordering::Relaxed),
        allocation_mismatch: ALLOCATION_MISMATCH.load(Ordering::Relaxed),
        rebinds: REBINDS.load(Ordering::Relaxed),
        forwarded_packets: FORWARDED_PACKETS.load(Ordering::Relaxed),
        forwarded_bytes: FORWARDED_BYTES.load(Ordering::Relaxed),
        dropped_packets: DROPPED_PACKETS.load(Ordering::Relaxed),
        rate_limited: RATE_LIMITED.load(Ordering::Relaxed),
        replay_rejected: REPLAY_REJECTED.load(Ordering::Relaxed),
        expired_allocations: EXPIRED_ALLOCATIONS.load(Ordering::Relaxed),
        listener_failures: LISTENER_FAILURES.load(Ordering::Relaxed),
    }
}

pub(crate) fn spawn(server_public_key: &str) {
    let Some(config) = Config::from_env(server_public_key) else {
        return;
    };
    ENABLED.store(true, Ordering::SeqCst);
    UDP_PORT.store(config.udp_port, Ordering::SeqCst);
    tokio::spawn(async move {
        loop {
            let error = match run(config.clone()).await {
                Ok(()) => "listener exited unexpectedly".to_owned(),
                Err(error) => error,
            };
            HEALTHY.store(false, Ordering::SeqCst);
            increment(&LISTENER_FAILURES);
            // The UDP data plane is optional and independently supervised. A
            // listener fault never tears down HBBR's reliable TCP/WS relay;
            // clients can fall back while this bounded retry loop recovers.
            log::error!("FastMedia Relay UDP listener unavailable: {error}; retrying");
            sleep(Duration::from_millis(500)).await;
        }
    });
}

#[derive(Clone)]
struct Config {
    bind: SocketAddr,
    udp_port: u16,
    relay_server: String,
    public_key: sign::PublicKey,
    max_allocations: usize,
    half_bind_ttl: Duration,
    idle_ttl: Duration,
    per_ip_packets_per_second: u64,
    global_packets_per_second: u64,
    per_ip_bytes_per_second: u64,
    global_bytes_per_second: u64,
}

impl Config {
    fn from_env(server_public_key: &str) -> Option<Self> {
        let udp_port =
            bounded_env_u64("STARRY_RELAY_FAST_MEDIA_UDP_PORT", 1, u16::MAX as u64)? as u16;
        let relay_server = std::env::var("STARRY_RELAY_PUBLIC_ENDPOINT")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty() && value.len() <= 256)?;
        let decoded = base64::decode(server_public_key).ok()?;
        let public_key = sign::PublicKey::from_slice(&decoded)?;
        let bind = std::env::var("STARRY_RELAY_FAST_MEDIA_BIND")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("0.0.0.0:{udp_port}"))
            .parse::<SocketAddr>()
            .ok()
            .filter(|address| address.port() == udp_port)?;
        Some(Self {
            bind,
            udp_port,
            relay_server,
            public_key,
            max_allocations: bounded_env_u64(
                "STARRY_RELAY_FAST_MEDIA_MAX_ALLOCATIONS",
                1,
                1_000_000,
            )
            .unwrap_or(DEFAULT_MAX_ALLOCATIONS as u64) as usize,
            half_bind_ttl: Duration::from_secs(
                bounded_env_u64("STARRY_RELAY_FAST_MEDIA_HALF_BIND_TTL_SECONDS", 2, 60)
                    .unwrap_or(DEFAULT_HALF_BIND_TTL_SECONDS),
            ),
            idle_ttl: Duration::from_secs(
                bounded_env_u64("STARRY_RELAY_FAST_MEDIA_IDLE_TTL_SECONDS", 5, 300)
                    .unwrap_or(DEFAULT_IDLE_TTL_SECONDS),
            ),
            per_ip_packets_per_second: bounded_env_u64(
                "STARRY_RELAY_FAST_MEDIA_PER_IP_PACKETS_PER_SECOND",
                100,
                1_000_000,
            )
            .unwrap_or(DEFAULT_PER_IP_PACKETS_PER_SECOND),
            global_packets_per_second: bounded_env_u64(
                "STARRY_RELAY_FAST_MEDIA_GLOBAL_PACKETS_PER_SECOND",
                1_000,
                10_000_000,
            )
            .unwrap_or(DEFAULT_GLOBAL_PACKETS_PER_SECOND),
            per_ip_bytes_per_second: bounded_env_u64(
                "STARRY_RELAY_FAST_MEDIA_PER_IP_BYTES_PER_SECOND",
                64 * 1024,
                1024 * 1024 * 1024,
            )
            .unwrap_or(DEFAULT_PER_IP_BYTES_PER_SECOND),
            global_bytes_per_second: bounded_env_u64(
                "STARRY_RELAY_FAST_MEDIA_GLOBAL_BYTES_PER_SECOND",
                1024 * 1024,
                8 * 1024 * 1024 * 1024,
            )
            .unwrap_or(DEFAULT_GLOBAL_BYTES_PER_SECOND),
        })
    }
}

fn bounded_env_u64(name: &str, minimum: u64, maximum: u64) -> Option<u64> {
    std::env::var(name)
        .ok()?
        .parse::<u64>()
        .ok()
        .filter(|value| (minimum..=maximum).contains(value))
}

async fn run(config: Config) -> Result<(), String> {
    sodiumoxide::init().map_err(|_| "crypto initialization failed".to_owned())?;
    let socket = UdpSocket::bind(config.bind)
        .await
        .map_err(|error| format!("bind failed: {error}"))?;
    let cookie_bytes = sodiumoxide::randombytes::randombytes(auth::KEYBYTES);
    let cookie_key = auth::Key::from_slice(&cookie_bytes)
        .ok_or_else(|| "cookie key initialization failed".to_owned())?;
    let mut engine = Engine::new(config, cookie_key);
    let mut buffer = vec![0_u8; MAX_AUTHORIZATION_BYTES + BIND_PREFIX_BYTES];
    let mut cleanup_tick = interval(Duration::from_secs(1));
    cleanup_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    HEALTHY.store(true, Ordering::SeqCst);
    log::info!("FastMedia Relay UDP listener is healthy");
    loop {
        tokio::select! {
            result = socket.recv_from(&mut buffer) => {
                let (size, source) = result.map_err(|error| format!("receive failed: {error}"))?;
                let now = Instant::now();
                let unix = unix_seconds();
                for outgoing in engine.handle(source, &buffer[..size], now, unix) {
                    if socket.send_to(&outgoing.bytes, outgoing.target).await.is_err() {
                        increment(&DROPPED_PACKETS);
                    }
                }
            }
            _ = cleanup_tick.tick() => {
                engine.cleanup(Instant::now(), unix_seconds());
            }
        }
    }
}

struct Engine {
    config: Config,
    cookie_key: auth::Key,
    allocations: HashMap<[u8; 16], Allocation>,
    cleanup_order: VecDeque<[u8; 16]>,
    per_ip: HashMap<IpAddr, TrafficLimit>,
    global: TrafficLimit,
}

impl Engine {
    fn new(config: Config, cookie_key: auth::Key) -> Self {
        let now = Instant::now();
        Self {
            global: TrafficLimit::new(
                config.global_packets_per_second,
                config.global_bytes_per_second,
                now,
            ),
            config,
            cookie_key,
            allocations: HashMap::new(),
            cleanup_order: VecDeque::new(),
            per_ip: HashMap::new(),
        }
    }

    fn handle(
        &mut self,
        source: SocketAddr,
        packet: &[u8],
        now: Instant,
        unix: u64,
    ) -> Vec<Outgoing> {
        let Some(header) = Header::parse(packet) else {
            increment(&DROPPED_PACKETS);
            return Vec::new();
        };
        if !self.allow_source(source.ip(), packet.len(), now) {
            increment(&RATE_LIMITED);
            return Vec::new();
        }
        match header.kind {
            1 => self.handle_hello(source, packet, header, unix),
            3 => self.handle_bind(source, packet, header, now, unix),
            5 => self.handle_media(source, packet, header, now, unix),
            _ => {
                increment(&DROPPED_PACKETS);
                Vec::new()
            }
        }
    }

    fn handle_hello(
        &self,
        source: SocketAddr,
        packet: &[u8],
        header: Header,
        unix: u64,
    ) -> Vec<Outgoing> {
        if packet.len() != HELLO_COOKIE_BYTES || packet[48..].iter().any(|byte| *byte != 0) {
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&packet[AKR_HEADER_BYTES..48]);
        let cookie = self.cookie(source, header, nonce, unix / COOKIE_EPOCH_SECONDS);
        let mut response = header.encode(2);
        response.extend_from_slice(&nonce);
        response.extend_from_slice(&cookie);
        increment(&HELLO_ACCEPTED);
        vec![Outgoing {
            target: source,
            bytes: response,
        }]
    }

    fn handle_bind(
        &mut self,
        source: SocketAddr,
        packet: &[u8],
        header: Header,
        now: Instant,
        unix: u64,
    ) -> Vec<Outgoing> {
        if packet.len() < BIND_PREFIX_BYTES {
            increment(&BIND_REJECTED);
            return Vec::new();
        }
        let authorization_len = u16::from_le_bytes([packet[56], packet[57]]) as usize;
        if authorization_len == 0
            || authorization_len > MAX_AUTHORIZATION_BYTES
            || packet.len() != BIND_PREFIX_BYTES + authorization_len
        {
            increment(&BIND_REJECTED);
            return Vec::new();
        }
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&packet[32..48]);
        let mut received_cookie = [0_u8; 8];
        received_cookie.copy_from_slice(&packet[48..56]);
        let current_epoch = unix / COOKIE_EPOCH_SECONDS;
        let cookie_valid = [current_epoch, current_epoch.saturating_sub(1)]
            .iter()
            .any(|epoch| self.cookie(source, header, nonce, *epoch) == received_cookie);
        if !cookie_valid {
            increment(&COOKIE_REJECTED);
            return Vec::new();
        }
        let signed = &packet[BIND_PREFIX_BYTES..];
        let payload = match sign::verify(signed, &self.config.public_key) {
            Ok(payload) => payload,
            Err(_) => {
                increment(&GRANT_REJECTED);
                return Vec::new();
            }
        };
        let grant = match FastRelayAuthorization::parse_from_bytes(&payload) {
            Ok(grant) => grant,
            Err(_) => {
                increment(&GRANT_REJECTED);
                return Vec::new();
            }
        };
        let invariant = match GrantInvariant::validated(
            &grant,
            header,
            &self.config.relay_server,
            self.config.udp_port,
            unix,
        ) {
            Ok(invariant) => invariant,
            Err(GrantError::Role) => {
                increment(&ROLE_MISMATCH);
                increment(&GRANT_REJECTED);
                return Vec::new();
            }
            Err(GrantError::Allocation) => {
                increment(&ALLOCATION_MISMATCH);
                increment(&GRANT_REJECTED);
                return Vec::new();
            }
            Err(GrantError::Other) => {
                increment(&GRANT_REJECTED);
                return Vec::new();
            }
        };

        if !self.allocations.contains_key(&header.allocation_id)
            && self.allocations.len() >= self.config.max_allocations
        {
            increment(&BIND_REJECTED);
            return Vec::new();
        }
        let new_allocation = !self.allocations.contains_key(&header.allocation_id);
        let allocation = self
            .allocations
            .entry(header.allocation_id)
            .or_insert_with(|| Allocation::new(header.session_id, invariant.clone(), now));
        if allocation.session_id != header.session_id {
            increment(&SESSION_MISMATCH);
            increment(&BIND_REJECTED);
            return Vec::new();
        }
        if allocation.invariant != invariant {
            increment(&ALLOCATION_MISMATCH);
            increment(&BIND_REJECTED);
            return Vec::new();
        }
        let existing_source = allocation
            .binding_mut(header.role)
            .as_ref()
            .map(|binding| binding.source);
        if let Some(existing_source) = existing_source {
            if existing_source == source {
                allocation.last_activity = now;
                return vec![Outgoing {
                    target: source,
                    bytes: header.encode(4),
                }];
            }
            if !allocation.allow_rebind(header.role, now) {
                increment(&RATE_LIMITED);
                increment(&BIND_REJECTED);
                return Vec::new();
            }
            increment(&REBINDS);
            // A cookie-authenticated rebind changes only the address. Keeping the
            // replay window and limiter prevents a port migration from resetting
            // either security boundary.
            if let Some(binding) = allocation.binding_mut(header.role).as_mut() {
                binding.source = source;
            }
        } else {
            *allocation.binding_mut(header.role) = Some(Binding {
                source,
                replay: ReplayWindow::default(),
                limiter: RoleLimit::new(grant.max_bitrate_kbps, now),
            });
        }
        allocation.last_activity = now;
        if new_allocation {
            self.cleanup_order.push_back(header.allocation_id);
        }
        self.update_active_counts();
        increment(&BIND_SUCCEEDED);
        vec![Outgoing {
            target: source,
            bytes: header.encode(4),
        }]
    }

    fn handle_media(
        &mut self,
        source: SocketAddr,
        packet: &[u8],
        header: Header,
        now: Instant,
        unix: u64,
    ) -> Vec<Outgoing> {
        let Some(allocation) = self.allocations.get_mut(&header.allocation_id) else {
            increment(&ALLOCATION_MISMATCH);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        };
        if allocation.session_id != header.session_id {
            increment(&SESSION_MISMATCH);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        if allocation.expired(now, unix, self.config.half_bind_ttl, self.config.idle_ttl)
            || packet.len() > allocation.invariant.max_datagram as usize
        {
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        if allocation.controller.is_none() || allocation.target.is_none() {
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        let payload = &packet[AKR_HEADER_BYTES..];
        let Some(sequence) = validate_akf1(payload, header.role, header.session_id) else {
            increment(&ROLE_MISMATCH);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        };
        let (source_binding, target) = match header.role {
            ROLE_CONTROLLER => (
                allocation.controller.as_mut(),
                allocation.target.as_ref().map(|binding| binding.source),
            ),
            ROLE_TARGET => (
                allocation.target.as_mut(),
                allocation.controller.as_ref().map(|binding| binding.source),
            ),
            _ => (None, None),
        };
        let (Some(source_binding), Some(target)) = (source_binding, target) else {
            increment(&DROPPED_PACKETS);
            return Vec::new();
        };
        if source_binding.source != source {
            increment(&ROLE_MISMATCH);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        if !source_binding.replay.accept(sequence) {
            increment(&REPLAY_REJECTED);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        if !source_binding.limiter.allow(packet.len(), now) {
            increment(&RATE_LIMITED);
            increment(&DROPPED_PACKETS);
            return Vec::new();
        }
        allocation.last_activity = now;
        increment(&FORWARDED_PACKETS);
        add(&FORWARDED_BYTES, payload.len() as u64);
        vec![Outgoing {
            target,
            bytes: payload.to_vec(),
        }]
    }

    fn allow_source(&mut self, ip: IpAddr, bytes: usize, now: Instant) -> bool {
        if !self.global.allow(bytes, now) {
            return false;
        }
        if !self.per_ip.contains_key(&ip) && self.per_ip.len() >= MAX_IP_BUCKETS {
            return false;
        }
        self.per_ip
            .entry(ip)
            .or_insert_with(|| {
                TrafficLimit::new(
                    self.config.per_ip_packets_per_second,
                    self.config.per_ip_bytes_per_second,
                    now,
                )
            })
            .allow(bytes, now)
    }

    fn cookie(&self, source: SocketAddr, header: Header, nonce: [u8; 16], epoch: u64) -> [u8; 8] {
        let mut canonical = Vec::with_capacity(96);
        canonical.extend_from_slice(b"starry-akr1-cookie-v1\0");
        match source.ip() {
            IpAddr::V4(ip) => canonical.extend_from_slice(&ip.octets()),
            IpAddr::V6(ip) => canonical.extend_from_slice(&ip.octets()),
        }
        canonical.extend_from_slice(&source.port().to_le_bytes());
        canonical.push(header.role);
        canonical.extend_from_slice(&header.session_id.to_le_bytes());
        canonical.extend_from_slice(&header.allocation_id);
        canonical.extend_from_slice(&nonce);
        canonical.extend_from_slice(&epoch.to_le_bytes());
        let tag = auth::authenticate(&canonical, &self.cookie_key);
        let mut cookie = [0_u8; 8];
        cookie.copy_from_slice(&tag.as_ref()[..8]);
        cookie
    }

    fn cleanup(&mut self, now: Instant, unix: u64) {
        for _ in 0..CLEANUP_PER_TICK {
            let Some(id) = self.cleanup_order.pop_front() else {
                break;
            };
            let expired = self
                .allocations
                .get(&id)
                .map(|allocation| {
                    allocation.expired(now, unix, self.config.half_bind_ttl, self.config.idle_ttl)
                })
                .unwrap_or(true);
            if expired {
                if self.allocations.remove(&id).is_some() {
                    increment(&EXPIRED_ALLOCATIONS);
                }
            } else {
                self.cleanup_order.push_back(id);
            }
        }
        self.update_active_counts();
        self.per_ip.retain(|_, limiter| {
            now.saturating_duration_since(limiter.last_seen) <= Duration::from_secs(60)
        });
    }

    fn update_active_counts(&self) {
        ACTIVE_ALLOCATIONS.store(self.allocations.len(), Ordering::Relaxed);
        ACTIVE_STREAMS.store(
            self.allocations
                .values()
                .filter(|allocation| allocation.controller.is_some() && allocation.target.is_some())
                .count(),
            Ordering::Relaxed,
        );
    }
}

#[derive(Clone, Copy)]
struct Header {
    kind: u8,
    role: u8,
    session_id: u64,
    allocation_id: [u8; 16],
}

impl Header {
    fn parse(packet: &[u8]) -> Option<Self> {
        if packet.len() < AKR_HEADER_BYTES
            || &packet[..4] != b"AKR1"
            || packet[4] != PROTOCOL_VERSION as u8
            || packet[7] != 0
            || !matches!(packet[5], 1 | 3 | 5)
            || !matches!(packet[6], ROLE_CONTROLLER | ROLE_TARGET)
        {
            return None;
        }
        let session_id = u64::from_le_bytes(packet[8..16].try_into().ok()?);
        if session_id == 0 {
            return None;
        }
        let allocation_id: [u8; 16] = packet[16..32].try_into().ok()?;
        if allocation_id.iter().all(|byte| *byte == 0) {
            return None;
        }
        Some(Self {
            kind: packet[5],
            role: packet[6],
            session_id,
            allocation_id,
        })
    }

    fn encode(self, kind: u8) -> Vec<u8> {
        let mut packet = Vec::with_capacity(AKR_HEADER_BYTES);
        packet.extend_from_slice(b"AKR1");
        packet.push(PROTOCOL_VERSION as u8);
        packet.push(kind);
        packet.push(self.role);
        packet.push(0);
        packet.extend_from_slice(&self.session_id.to_le_bytes());
        packet.extend_from_slice(&self.allocation_id);
        packet
    }
}

#[derive(Clone, Eq, PartialEq)]
struct GrantInvariant {
    session_uuid_digest: [u8; 32],
    expires_at: u64,
    max_bitrate_kbps: u32,
    max_datagram: u32,
}

enum GrantError {
    Role,
    Allocation,
    Other,
}

impl GrantInvariant {
    fn validated(
        grant: &FastRelayAuthorization,
        header: Header,
        relay_server: &str,
        udp_port: u16,
        unix: u64,
    ) -> Result<Self, GrantError> {
        if grant.relay_endpoint_role != u32::from(header.role) {
            return Err(GrantError::Role);
        }
        if grant.relay_allocation_id.as_ref() != header.allocation_id {
            return Err(GrantError::Allocation);
        }
        if grant.version != PROTOCOL_VERSION
            || grant.session_uuid.is_empty()
            || grant.session_uuid.len() > 128
            || !grant.allow_fast_media_v1
            || !grant.allow_fast_compat
            || grant.relay_udp_protocol != PROTOCOL_VERSION
            || !grant.relay_server.eq_ignore_ascii_case(relay_server)
            || grant.relay_udp_port != u32::from(udp_port)
            || !(MIN_RELAY_DATAGRAM..=MAX_RELAY_DATAGRAM).contains(&grant.relay_max_datagram)
            || !(1_000..=200_000).contains(&grant.max_bitrate_kbps)
            || grant.expires_at <= unix
            || grant.expires_at > unix.saturating_add(MAX_GRANT_TTL_SECONDS)
        {
            return Err(GrantError::Other);
        }
        let digest = Sha256::digest(grant.session_uuid.as_bytes());
        let mut session_uuid_digest = [0_u8; 32];
        session_uuid_digest.copy_from_slice(&digest);
        Ok(Self {
            session_uuid_digest,
            expires_at: grant.expires_at,
            max_bitrate_kbps: grant.max_bitrate_kbps,
            max_datagram: grant.relay_max_datagram,
        })
    }
}

struct Allocation {
    session_id: u64,
    invariant: GrantInvariant,
    created: Instant,
    last_activity: Instant,
    controller: Option<Binding>,
    target: Option<Binding>,
    controller_rebind: RebindWindow,
    target_rebind: RebindWindow,
}

impl Allocation {
    fn new(session_id: u64, invariant: GrantInvariant, now: Instant) -> Self {
        Self {
            session_id,
            invariant,
            created: now,
            last_activity: now,
            controller: None,
            target: None,
            controller_rebind: RebindWindow::new(now),
            target_rebind: RebindWindow::new(now),
        }
    }

    fn binding_mut(&mut self, role: u8) -> &mut Option<Binding> {
        if role == ROLE_CONTROLLER {
            &mut self.controller
        } else {
            &mut self.target
        }
    }

    fn allow_rebind(&mut self, role: u8, now: Instant) -> bool {
        if role == ROLE_CONTROLLER {
            self.controller_rebind.allow(now)
        } else {
            self.target_rebind.allow(now)
        }
    }

    fn expired(
        &self,
        now: Instant,
        unix: u64,
        half_bind_ttl: Duration,
        idle_ttl: Duration,
    ) -> bool {
        self.invariant.expires_at <= unix
            || now.saturating_duration_since(self.created)
                > Duration::from_secs(MAX_GRANT_TTL_SECONDS)
            || now.saturating_duration_since(self.last_activity) > idle_ttl
            || ((self.controller.is_none() || self.target.is_none())
                && now.saturating_duration_since(self.created) > half_bind_ttl)
    }
}

struct Binding {
    source: SocketAddr,
    replay: ReplayWindow,
    limiter: RoleLimit,
}

struct RebindWindow {
    started: Instant,
    count: u32,
}

impl RebindWindow {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            count: 0,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.started) >= Duration::from_secs(60) {
            self.started = now;
            self.count = 0;
        }
        if self.count >= REBINDS_PER_MINUTE {
            return false;
        }
        self.count = self.count.saturating_add(1);
        true
    }
}

#[derive(Default)]
struct ReplayWindow {
    maximum: u64,
    bitmap: u128,
}

impl ReplayWindow {
    fn accept(&mut self, sequence: u64) -> bool {
        if sequence == 0 {
            return false;
        }
        if sequence > self.maximum {
            let shift = sequence.saturating_sub(self.maximum);
            self.bitmap = if shift >= 128 {
                1
            } else {
                (self.bitmap << shift) | 1
            };
            self.maximum = sequence;
            return true;
        }
        let distance = self.maximum - sequence;
        if distance >= 128 {
            return false;
        }
        let bit = 1_u128 << distance;
        if self.bitmap & bit != 0 {
            return false;
        }
        self.bitmap |= bit;
        true
    }
}

struct RoleLimit {
    bytes: TokenBucket,
}

impl RoleLimit {
    fn new(source_kbps: u32, now: Instant) -> Self {
        let wire_kbps = u64::from(source_kbps)
            .saturating_mul(145)
            .saturating_add(99)
            / 100;
        let bytes_per_second = wire_kbps.saturating_mul(1_000).saturating_add(7) / 8;
        let burst = (256 * 1024_u64).max(bytes_per_second.saturating_mul(50) / 1_000);
        Self {
            bytes: TokenBucket::new(bytes_per_second, burst, now),
        }
    }

    fn allow(&mut self, bytes: usize, now: Instant) -> bool {
        self.bytes.allow(bytes as u64, now)
    }
}

struct TrafficLimit {
    packets: TokenBucket,
    bytes: TokenBucket,
    last_seen: Instant,
}

impl TrafficLimit {
    fn new(packets_per_second: u64, bytes_per_second: u64, now: Instant) -> Self {
        Self {
            packets: TokenBucket::new(packets_per_second, packets_per_second.max(1), now),
            bytes: TokenBucket::new(bytes_per_second, bytes_per_second.max(1), now),
            last_seen: now,
        }
    }

    fn allow(&mut self, bytes: usize, now: Instant) -> bool {
        self.last_seen = now;
        self.packets.allow(1, now) && self.bytes.allow(bytes as u64, now)
    }
}

struct TokenBucket {
    rate: u64,
    capacity: u64,
    available: f64,
    updated: Instant,
}

impl TokenBucket {
    fn new(rate: u64, capacity: u64, now: Instant) -> Self {
        Self {
            rate: rate.max(1),
            capacity: capacity.max(1),
            available: capacity.max(1) as f64,
            updated: now,
        }
    }

    fn allow(&mut self, amount: u64, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        self.available = (self.available + elapsed * self.rate as f64).min(self.capacity as f64);
        self.updated = now;
        if self.available < amount as f64 {
            return false;
        }
        self.available -= amount as f64;
        true
    }
}

struct Outgoing {
    target: SocketAddr,
    bytes: Vec<u8>,
}

fn validate_akf1(payload: &[u8], role: u8, session_id: u64) -> Option<u64> {
    if payload.len() < MIN_AKF1_BYTES
        || &payload[..4] != b"AKF1"
        || payload[4] != 1
        || payload[5] != role.saturating_sub(1)
        || u64::from_le_bytes(payload[6..14].try_into().ok()?) != session_id
    {
        return None;
    }
    let sequence = u64::from_le_bytes(payload[14..22].try_into().ok()?);
    (sequence > 0).then_some(sequence)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(public_key: sign::PublicKey) -> Config {
        Config {
            bind: "127.0.0.1:22119".parse().unwrap(),
            udp_port: 22119,
            relay_server: "relay.example:21117".to_owned(),
            public_key,
            max_allocations: 8,
            half_bind_ttl: Duration::from_secs(10),
            idle_ttl: Duration::from_secs(30),
            per_ip_packets_per_second: 100_000,
            global_packets_per_second: 100_000,
            per_ip_bytes_per_second: 100_000_000,
            global_bytes_per_second: 100_000_000,
        }
    }

    fn header(kind: u8, role: u8) -> Header {
        Header {
            kind,
            role,
            session_id: 77,
            allocation_id: [3; 16],
        }
    }

    fn grant(role: u8, expires_at: u64) -> FastRelayAuthorization {
        FastRelayAuthorization {
            version: 1,
            session_uuid: "session-test".to_owned(),
            expires_at,
            allow_fast_compat: true,
            allow_fast_media_v1: true,
            max_bitrate_kbps: 50_000,
            relay_udp_protocol: 1,
            relay_server: "relay.example:21117".to_owned(),
            relay_udp_port: 22119,
            relay_allocation_id: vec![3; 16].into(),
            relay_max_datagram: 1_200,
            relay_endpoint_role: u32::from(role),
            ..Default::default()
        }
    }

    fn hello(role: u8, nonce: [u8; 16]) -> Vec<u8> {
        let mut packet = header(1, role).encode(1);
        packet.extend_from_slice(&nonce);
        packet.extend_from_slice(&[0; 8]);
        packet
    }

    fn bind(role: u8, nonce: [u8; 16], cookie: [u8; 8], signed: &[u8]) -> Vec<u8> {
        let mut packet = header(3, role).encode(3);
        packet.extend_from_slice(&nonce);
        packet.extend_from_slice(&cookie);
        packet.extend_from_slice(&(signed.len() as u16).to_le_bytes());
        packet.extend_from_slice(signed);
        packet
    }

    fn akf1(role: u8, sequence: u64) -> Vec<u8> {
        let mut payload = vec![0_u8; MIN_AKF1_BYTES];
        payload[..4].copy_from_slice(b"AKF1");
        payload[4] = 1;
        payload[5] = role - 1;
        payload[6..14].copy_from_slice(&77_u64.to_le_bytes());
        payload[14..22].copy_from_slice(&sequence.to_le_bytes());
        payload
    }

    fn bind_role(
        engine: &mut Engine,
        secret: &sign::SecretKey,
        source: SocketAddr,
        role: u8,
        now: Instant,
        unix: u64,
    ) {
        bind_role_with_expiry(engine, secret, source, role, now, unix, unix + 90);
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_role_with_expiry(
        engine: &mut Engine,
        secret: &sign::SecretKey,
        source: SocketAddr,
        role: u8,
        now: Instant,
        unix: u64,
        expires_at: u64,
    ) {
        let nonce = [role; 16];
        let cookie_reply = engine.handle(source, &hello(role, nonce), now, unix);
        assert_eq!(cookie_reply.len(), 1);
        assert_eq!(cookie_reply[0].bytes.len(), HELLO_COOKIE_BYTES);
        let cookie: [u8; 8] = cookie_reply[0].bytes[48..56].try_into().unwrap();
        let payload = grant(role, expires_at).write_to_bytes().unwrap();
        let signed = sign::sign(&payload, secret);
        let bound = engine.handle(source, &bind(role, nonce, cookie, &signed), now, unix);
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].bytes[5], 4);
    }

    fn bind_grant(
        engine: &mut Engine,
        secret: &sign::SecretKey,
        source: SocketAddr,
        role: u8,
        grant: &FastRelayAuthorization,
        now: Instant,
        unix: u64,
    ) -> Vec<Outgoing> {
        let nonce = [role.saturating_add(10); 16];
        let cookie_reply = engine.handle(source, &hello(role, nonce), now, unix);
        let cookie: [u8; 8] = cookie_reply[0].bytes[48..56].try_into().unwrap();
        let signed = sign::sign(&grant.write_to_bytes().unwrap(), secret);
        engine.handle(source, &bind(role, nonce, cookie, &signed), now, unix)
    }

    #[test]
    fn cookie_bind_two_roles_forward_and_replay_fail_closed() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[9; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let controller: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();
        bind_role(&mut engine, &secret, controller, ROLE_CONTROLLER, now, unix);
        bind_role(&mut engine, &secret, target, ROLE_TARGET, now, unix);

        let payload = akf1(ROLE_TARGET, 1);
        let mut media = header(5, ROLE_TARGET).encode(5);
        media.extend_from_slice(&payload);
        let forwarded = engine.handle(target, &media, now, unix);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].target, controller);
        assert_eq!(forwarded[0].bytes, payload);
        assert!(engine.handle(target, &media, now, unix).is_empty());
    }

    #[test]
    fn wrong_role_grant_cookie_and_inner_session_are_rejected() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[8; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let source: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let nonce = [1; 16];
        let reply = engine.handle(source, &hello(ROLE_CONTROLLER, nonce), now, unix);
        let cookie: [u8; 8] = reply[0].bytes[48..56].try_into().unwrap();

        let wrong_role = grant(ROLE_TARGET, unix + 90).write_to_bytes().unwrap();
        assert!(engine
            .handle(
                source,
                &bind(
                    ROLE_CONTROLLER,
                    nonce,
                    cookie,
                    &sign::sign(&wrong_role, &secret),
                ),
                now,
                unix,
            )
            .is_empty());
        let mut wrong_cookie = cookie;
        wrong_cookie[0] ^= 1;
        let correct = grant(ROLE_CONTROLLER, unix + 90).write_to_bytes().unwrap();
        assert!(engine
            .handle(
                source,
                &bind(
                    ROLE_CONTROLLER,
                    nonce,
                    wrong_cookie,
                    &sign::sign(&correct, &secret),
                ),
                now,
                unix,
            )
            .is_empty());
    }

    #[test]
    fn tamper_expiry_uuid_relay_allocation_and_session_mismatches_fail_closed() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[6; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let controller: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();

        let mut expired = grant(ROLE_CONTROLLER, unix);
        assert!(bind_grant(
            &mut engine,
            &secret,
            controller,
            ROLE_CONTROLLER,
            &expired,
            now,
            unix,
        )
        .is_empty());
        expired.expires_at = unix + 90;
        expired.relay_server = "another-relay.example:21117".to_owned();
        assert!(bind_grant(
            &mut engine,
            &secret,
            controller,
            ROLE_CONTROLLER,
            &expired,
            now,
            unix,
        )
        .is_empty());
        expired.relay_server = "relay.example:21117".to_owned();
        expired.relay_allocation_id = vec![4; 16].into();
        assert!(bind_grant(
            &mut engine,
            &secret,
            controller,
            ROLE_CONTROLLER,
            &expired,
            now,
            unix,
        )
        .is_empty());

        let nonce = [31; 16];
        let reply = engine.handle(controller, &hello(ROLE_CONTROLLER, nonce), now, unix);
        let cookie: [u8; 8] = reply[0].bytes[48..56].try_into().unwrap();
        let mut tampered = sign::sign(
            &grant(ROLE_CONTROLLER, unix + 90).write_to_bytes().unwrap(),
            &secret,
        );
        tampered[0] ^= 1;
        assert!(engine
            .handle(
                controller,
                &bind(ROLE_CONTROLLER, nonce, cookie, &tampered),
                now,
                unix,
            )
            .is_empty());

        bind_role(&mut engine, &secret, controller, ROLE_CONTROLLER, now, unix);
        let mut wrong_uuid = grant(ROLE_TARGET, unix + 90);
        wrong_uuid.session_uuid = "another-session".to_owned();
        assert!(bind_grant(
            &mut engine,
            &secret,
            target,
            ROLE_TARGET,
            &wrong_uuid,
            now,
            unix,
        )
        .is_empty());
        bind_role(&mut engine, &secret, target, ROLE_TARGET, now, unix);

        let mut wrong_inner = header(5, ROLE_TARGET).encode(5);
        let mut payload = akf1(ROLE_TARGET, 1);
        payload[6..14].copy_from_slice(&88_u64.to_le_bytes());
        wrong_inner.extend_from_slice(&payload);
        assert!(engine.handle(target, &wrong_inner, now, unix).is_empty());

        let mut wrong_outer_header = header(5, ROLE_TARGET);
        wrong_outer_header.session_id = 88;
        let mut wrong_outer = wrong_outer_header.encode(5);
        wrong_outer.extend_from_slice(&akf1(ROLE_TARGET, 2));
        assert!(engine.handle(target, &wrong_outer, now, unix).is_empty());

        let mut wrong_allocation_header = header(5, ROLE_TARGET);
        wrong_allocation_header.allocation_id = [4; 16];
        let mut wrong_allocation = wrong_allocation_header.encode(5);
        wrong_allocation.extend_from_slice(&akf1(ROLE_TARGET, 3));
        assert!(engine
            .handle(target, &wrong_allocation, now, unix)
            .is_empty());
    }

    #[test]
    fn fresh_cookie_allows_300_to_1200ms_source_rebind_and_preserves_replay_state() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[7; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let old: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let after_300ms: SocketAddr = "192.0.2.10:40123".parse().unwrap();
        let after_1200ms: SocketAddr = "192.0.2.10:40234".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();
        bind_role(&mut engine, &secret, old, ROLE_CONTROLLER, now, unix);
        bind_role(&mut engine, &secret, target, ROLE_TARGET, now, unix);
        bind_role(
            &mut engine,
            &secret,
            after_300ms,
            ROLE_CONTROLLER,
            now + Duration::from_millis(300),
            unix,
        );

        let mut old_media = header(5, ROLE_CONTROLLER).encode(5);
        old_media.extend_from_slice(&akf1(ROLE_CONTROLLER, 1));
        assert!(engine.handle(old, &old_media, now, unix).is_empty());
        let forwarded = engine.handle(
            after_300ms,
            &old_media,
            now + Duration::from_millis(300),
            unix,
        );
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].target, target);

        bind_role_with_expiry(
            &mut engine,
            &secret,
            after_1200ms,
            ROLE_CONTROLLER,
            now + Duration::from_millis(1_200),
            unix + 1,
            unix + 90,
        );
        let mut second_media = header(5, ROLE_CONTROLLER).encode(5);
        second_media.extend_from_slice(&akf1(ROLE_CONTROLLER, 2));
        assert!(engine
            .handle(
                after_300ms,
                &second_media,
                now + Duration::from_millis(1_200),
                unix + 1,
            )
            .is_empty());
        let forwarded = engine.handle(
            after_1200ms,
            &second_media,
            now + Duration::from_millis(1_200),
            unix + 1,
        );
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].target, target);
        assert!(engine
            .handle(
                after_1200ms,
                &old_media,
                now + Duration::from_millis(1_200),
                unix + 1,
            )
            .is_empty());
    }

    #[test]
    fn half_bound_idle_and_absolute_ttl_cleanup_are_bounded() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let now = Instant::now();
        let unix = 1_800_000_000;
        let controller: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();

        let key = auth::Key::from_slice(&[4; auth::KEYBYTES]).unwrap();
        let mut half_bound = Engine::new(config(public), key);
        bind_role(
            &mut half_bound,
            &secret,
            controller,
            ROLE_CONTROLLER,
            now,
            unix,
        );
        assert_eq!(half_bound.allocations.len(), 1);
        half_bound.cleanup(now + Duration::from_secs(11), unix);
        assert!(half_bound.allocations.is_empty());

        let key = auth::Key::from_slice(&[5; auth::KEYBYTES]).unwrap();
        let mut idle = Engine::new(config(public), key);
        bind_role(&mut idle, &secret, controller, ROLE_CONTROLLER, now, unix);
        bind_role(&mut idle, &secret, target, ROLE_TARGET, now, unix);
        idle.cleanup(now + Duration::from_secs(31), unix);
        assert!(idle.allocations.is_empty());

        let key = auth::Key::from_slice(&[10; auth::KEYBYTES]).unwrap();
        let mut absolute = Engine::new(config(public), key);
        bind_role_with_expiry(
            &mut absolute,
            &secret,
            controller,
            ROLE_CONTROLLER,
            now,
            unix,
            unix + MAX_GRANT_TTL_SECONDS,
        );
        bind_role_with_expiry(
            &mut absolute,
            &secret,
            target,
            ROLE_TARGET,
            now,
            unix,
            unix + MAX_GRANT_TTL_SECONDS,
        );
        absolute.cleanup(now + Duration::from_secs(MAX_GRANT_TTL_SECONDS + 1), unix);
        assert!(absolute.allocations.is_empty());
    }

    #[test]
    fn sustained_wire_overlimit_drops_media_and_recovers_with_bounded_tokens() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[11; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let controller: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();
        let mut controller_grant = grant(ROLE_CONTROLLER, unix + 90);
        controller_grant.max_bitrate_kbps = 1_000;
        let mut target_grant = grant(ROLE_TARGET, unix + 90);
        target_grant.max_bitrate_kbps = 1_000;
        assert_eq!(
            bind_grant(
                &mut engine,
                &secret,
                controller,
                ROLE_CONTROLLER,
                &controller_grant,
                now,
                unix,
            )
            .len(),
            1
        );
        assert_eq!(
            bind_grant(
                &mut engine,
                &secret,
                target,
                ROLE_TARGET,
                &target_grant,
                now,
                unix,
            )
            .len(),
            1
        );

        let mut accepted = 0;
        let mut dropped = 0;
        for sequence in 1..=512 {
            let mut media = header(5, ROLE_TARGET).encode(5);
            media.extend_from_slice(&akf1(ROLE_TARGET, sequence));
            media.resize(1_200, 0x5a);
            if engine.handle(target, &media, now, unix).is_empty() {
                dropped += 1;
            } else {
                accepted += 1;
            }
        }
        assert!(accepted > 0);
        assert!(
            dropped > 0,
            "a sustained burst must exhaust the wire bucket"
        );

        let mut recovered = header(5, ROLE_TARGET).encode(5);
        recovered.extend_from_slice(&akf1(ROLE_TARGET, 1_000));
        recovered.resize(1_200, 0x5a);
        assert_eq!(
            engine
                .handle(target, &recovered, now + Duration::from_secs(1), unix + 1)
                .len(),
            1,
            "the bounded token bucket must recover without resetting the allocation"
        );
    }

    #[test]
    fn authorization_and_datagram_bounds_fail_closed() {
        sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = auth::Key::from_slice(&[12; auth::KEYBYTES]).unwrap();
        let mut engine = Engine::new(config(public), key);
        let now = Instant::now();
        let unix = 1_800_000_000;
        let controller: SocketAddr = "192.0.2.10:40000".parse().unwrap();
        let target: SocketAddr = "198.51.100.20:50000".parse().unwrap();

        for invalid_max_datagram in [MIN_RELAY_DATAGRAM - 1, MAX_RELAY_DATAGRAM + 1] {
            let mut invalid = grant(ROLE_CONTROLLER, unix + 90);
            invalid.relay_max_datagram = invalid_max_datagram;
            assert!(bind_grant(
                &mut engine,
                &secret,
                controller,
                ROLE_CONTROLLER,
                &invalid,
                now,
                unix,
            )
            .is_empty());
        }

        let nonce = [0x44; 16];
        let reply = engine.handle(controller, &hello(ROLE_CONTROLLER, nonce), now, unix);
        let cookie: [u8; 8] = reply[0].bytes[48..56].try_into().unwrap();
        assert!(engine
            .handle(
                controller,
                &bind(
                    ROLE_CONTROLLER,
                    nonce,
                    cookie,
                    &vec![0x5a; MAX_AUTHORIZATION_BYTES + 1],
                ),
                now,
                unix,
            )
            .is_empty());

        bind_role(&mut engine, &secret, controller, ROLE_CONTROLLER, now, unix);
        bind_role(&mut engine, &secret, target, ROLE_TARGET, now, unix);
        let mut oversized_media = header(5, ROLE_TARGET).encode(5);
        oversized_media.extend_from_slice(&akf1(ROLE_TARGET, 1));
        oversized_media.resize(1_201, 0x5a);
        assert!(engine
            .handle(target, &oversized_media, now, unix)
            .is_empty());
    }
}
