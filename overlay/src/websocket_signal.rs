mod relay_health;
mod routing;
mod session;

use crate::starry_config::{self, WebSocketSignalConfig};
use hbb_common::{
    log,
    protobuf::Message as _,
    rendezvous_proto::RendezvousMessage,
    tokio::{net::TcpStream, sync::mpsc},
};
use ipnetwork::IpNetwork;
use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;

pub(crate) use relay_health::{runtime_snapshot as health_runtime_snapshot, RuntimeHealthSnapshot};
pub(crate) use routing::{SessionRoute, SessionToken};
pub(crate) use session::WsWriteTransport;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct PreparedWebSocketSignal {
    config: WebSocketSignalConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RelayRequirement {
    NativeOnly,
    WebSocketOnly,
    Mixed,
}

impl RelayRequirement {
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        match value.map(str::to_ascii_lowercase).as_deref() {
            None | Some("native") => Some(Self::NativeOnly),
            Some("wss" | "websocket") => Some(Self::WebSocketOnly),
            Some("mixed") => Some(Self::Mixed),
            _ => None,
        }
    }
}

pub(crate) fn config() -> WebSocketSignalConfig {
    starry_config::snapshot()
        .map(|config| config.websocket_signal.clone())
        .unwrap_or_default()
}

pub(crate) fn reconfigure() -> String {
    let config = config();
    match prepare(&config).and_then(activate_prepared) {
        Ok(ack) => ack.detail,
        Err(err) => {
            format!("WebSocket Signal reload rejected; retained last-known-good state: {err}")
        }
    }
}

pub(crate) fn prepare(config: &WebSocketSignalConfig) -> Result<PreparedWebSocketSignal, String> {
    Ok(PreparedWebSocketSignal {
        config: config.clone(),
    })
}

pub(crate) fn activate_prepared(
    prepared: PreparedWebSocketSignal,
) -> Result<starry_config::SubsystemAck, String> {
    let config = prepared.config;
    let drained = if config.enabled {
        (0, 0)
    } else {
        routing::drain_all_now()?
    };
    let health = relay_health::reconfigure(config.enabled, &config.relay_health)?;
    if drained.0 > 0 || drained.1 > 0 {
        log::info!(
            "Drained {} WebSocket signal sessions and {} admitted connections before configuration disable acknowledgement",
            drained.0,
            drained.1
        );
    }
    Ok(starry_config::SubsystemAck {
        subsystem: "websocket_signal".to_owned(),
        accepted: true,
        detail: health,
    })
}

pub(crate) fn relay_ready() -> bool {
    relay_health::ready()
}

pub(crate) fn health_snapshot_id() -> u64 {
    relay_health::snapshot_id()
}

pub(crate) fn eligible_relays(
    configured_relays: &[String],
    native_online: &[String],
    requirement: RelayRequirement,
) -> Vec<String> {
    relay_health::eligible_relays(configured_relays, native_online, requirement)
}

pub(crate) fn next_connection_id() -> u64 {
    NEXT_CONNECTION_ID.fetch_add(1, Ordering::SeqCst)
}

pub(crate) fn inspect_upgrade(
    uri: &http::Uri,
    headers: &http::HeaderMap,
    direct_addr: SocketAddr,
    config: &WebSocketSignalConfig,
) -> Result<SocketAddr, String> {
    if uri.path() != "/ws/id" || uri.query().is_some() {
        return Err("WebSocket Signal requires the exact /ws/id path".to_owned());
    }
    if let Some(origin) = headers.get(http::header::ORIGIN) {
        let origin = origin
            .to_str()
            .map_err(|_| "Origin header is not valid ASCII".to_owned())?;
        if !config.allowed_origins.iter().any(|item| item == origin) {
            return Err("Origin is not explicitly allowed".to_owned());
        }
    }

    // The official listener prefers a dual-stack IPv6 socket. Linux reports
    // IPv4 proxy connections accepted by that socket as ::ffff:a.b.c.d, while
    // operators correctly configure the proxy as an IPv4 CIDR. Compare both
    // representations without broadening any configured trust range.
    let direct_ip = match direct_addr.ip() {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4),
        IpAddr::V4(_) => None,
    };
    let trusted = config.trusted_proxies.iter().any(|cidr| {
        cidr.parse::<IpNetwork>()
            .map(|network| {
                network.contains(direct_addr.ip())
                    || direct_ip
                        .map(|normalized| network.contains(normalized))
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    });
    if !trusted {
        return Ok(direct_addr);
    }

    let forwarded_ip = if let Some(value) = headers.get("X-Real-IP") {
        let value = value
            .to_str()
            .map_err(|_| "X-Real-IP is not valid ASCII".to_owned())?
            .trim();
        if value.contains(',') || value.len() > 64 {
            return Err("X-Real-IP is malformed".to_owned());
        }
        Some(
            value
                .parse::<IpAddr>()
                .map_err(|_| "X-Real-IP is not an IP address".to_owned())?,
        )
    } else if let Some(value) = headers.get("X-Forwarded-For") {
        let value = value
            .to_str()
            .map_err(|_| "X-Forwarded-For is not valid ASCII".to_owned())?;
        if value.len() > 2_048 {
            return Err("X-Forwarded-For is too long".to_owned());
        }
        let values: Vec<&str> = value.split(',').map(str::trim).collect();
        if values.is_empty() || values.len() > 16 || values.iter().any(|value| value.is_empty()) {
            return Err("X-Forwarded-For has an invalid chain length".to_owned());
        }
        let parsed: Result<Vec<IpAddr>, _> = values.iter().map(|value| value.parse()).collect();
        Some(parsed.map_err(|_| "X-Forwarded-For contains a non-IP entry".to_owned())?[0])
    } else {
        None
    };
    Ok(SocketAddr::new(
        forwarded_ip.unwrap_or_else(|| direct_addr.ip()),
        direct_addr.port(),
    ))
}

pub(crate) fn transport(
    connection_id: u64,
    capacity: usize,
) -> (WsWriteTransport, mpsc::Receiver<PrivateOutboundFrame>) {
    let (writer, receiver) = session::WsWriteTransport::channel(connection_id, capacity);
    // The private type is exposed only inside this parent module and its injected caller.
    (writer, receiver)
}

pub(crate) type PrivateOutboundFrame = session::OutboundFrame;

pub(crate) async fn writer_loop(
    sink: hbb_common::futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    receiver: mpsc::Receiver<PrivateOutboundFrame>,
    writer: WsWriteTransport,
) {
    session::writer_loop(sink, receiver, writer).await;
}

pub(crate) async fn register_connection(
    route_addr: SocketAddr,
    effective_addr: SocketAddr,
    connection_id: u64,
    writer: WsWriteTransport,
) {
    routing::register_connection(route_addr, effective_addr, connection_id, writer).await;
}

pub(crate) async fn remove_connection(route_addr: SocketAddr, connection_id: u64) {
    routing::remove_connection(route_addr, connection_id).await;
}

pub(crate) async fn connection_effective(route_addr: SocketAddr) -> Option<SocketAddr> {
    routing::connection_effective(route_addr).await
}

pub(crate) async fn is_websocket_route(route_addr: SocketAddr) -> bool {
    routing::is_websocket_route(route_addr).await
}

pub(crate) async fn allow_registration(effective_ip: IpAddr, limit: usize) -> bool {
    routing::allow_registration(effective_ip, limit).await
}

pub(crate) async fn capacity_available(
    peer_id: &str,
    effective_ip: IpAddr,
    config: &WebSocketSignalConfig,
) -> bool {
    routing::capacity_available(
        peer_id,
        effective_ip,
        config.max_sessions,
        config.max_sessions_per_effective_ip,
    )
    .await
}

pub(crate) async fn bind(
    peer_id: String,
    writer: WsWriteTransport,
    effective_ip: IpAddr,
    route_addr: SocketAddr,
    config: &WebSocketSignalConfig,
) -> Result<SessionToken, String> {
    routing::bind(
        peer_id,
        writer,
        effective_ip,
        route_addr,
        config.max_sessions,
        config.max_sessions_per_effective_ip,
    )
    .await
}

pub(crate) async fn route(peer_id: &str) -> Option<SessionRoute> {
    routing::route(peer_id).await
}

pub(crate) async fn send_to_peer(peer_id: &str, message: &RendezvousMessage) -> bool {
    let Ok(bytes) = message.write_to_bytes() else {
        return false;
    };
    routing::try_send(peer_id, bytes).await
}

pub(crate) async fn remove_session(token: &SessionToken, reason: &str) -> bool {
    routing::remove_if_current(&token.peer_id, token.generation, reason).await
}

pub(crate) async fn native_registration(peer_id: &str) -> bool {
    routing::native_registration(peer_id).await
}

pub(crate) async fn status(native_online: &[String]) -> String {
    use std::fmt::Write as _;

    let config = config();
    let routing = routing::status().await;
    let native: std::collections::HashSet<String> = native_online
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let mut output = String::new();
    let _ = writeln!(output, "enabled: {}", config.enabled);
    let _ = writeln!(output, "relay_ready: {}", relay_ready());
    let _ = writeln!(output, "sessions: {}", routing.sessions);
    let _ = writeln!(output, "draining: {}", routing.draining);
    let _ = writeln!(output, "registered_total: {}", routing.registered);
    let _ = writeln!(output, "replaced_total: {}", routing.replaced);
    let _ = writeln!(output, "native_evictions_total: {}", routing.evicted);
    let _ = writeln!(output, "timeouts_total: {}", routing.timed_out);
    let _ = writeln!(output, "slow_consumers_total: {}", routing.slow_consumers);
    for endpoint in relay_health::snapshots() {
        let native_status = if native.contains(&endpoint.relay.to_ascii_lowercase()) {
            "healthy"
        } else {
            "unavailable"
        };
        let _ = writeln!(
            output,
            "relay {} native={} websocket={} last_success_age_s={:?} last_failure_age_s={:?} last_error={}",
            endpoint.relay,
            native_status,
            endpoint.status,
            endpoint.last_success_age_seconds,
            endpoint.last_failure_age_seconds,
            endpoint.last_error.as_deref().unwrap_or("none")
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WebSocketSignalConfig {
        WebSocketSignalConfig::default()
    }

    #[test]
    fn untrusted_peer_cannot_spoof_forwarded_ip() {
        let uri: http::Uri = "/ws/id".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Real-IP", "198.51.100.7".parse().unwrap());
        let direct: SocketAddr = "203.0.113.9:32100".parse().unwrap();
        assert_eq!(
            inspect_upgrade(&uri, &headers, direct, &config()).unwrap(),
            direct
        );
    }

    #[test]
    fn trusted_proxy_preserves_unique_connection_port() {
        let uri: http::Uri = "/ws/id".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Real-IP", "198.51.100.7".parse().unwrap());
        let direct: SocketAddr = "127.0.0.1:32100".parse().unwrap();
        assert_eq!(
            inspect_upgrade(&uri, &headers, direct, &config()).unwrap(),
            "198.51.100.7:32100".parse().unwrap()
        );
    }

    #[test]
    fn ipv4_mapped_dual_stack_proxy_matches_ipv4_trust_range() {
        let uri: http::Uri = "/ws/id".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Real-IP", "198.51.100.8".parse().unwrap());
        let direct: SocketAddr = "[::ffff:127.0.0.1]:32101".parse().unwrap();
        assert_eq!(
            inspect_upgrade(&uri, &headers, direct, &config()).unwrap(),
            "198.51.100.8:32101".parse().unwrap()
        );
    }

    #[test]
    fn origin_requires_an_exact_allow_list_match() {
        let uri: http::Uri = "/ws/id".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::ORIGIN,
            "https://console.example.com".parse().unwrap(),
        );
        let direct: SocketAddr = "127.0.0.1:32100".parse().unwrap();
        assert!(inspect_upgrade(&uri, &headers, direct, &config()).is_err());
        let mut allowed = config();
        allowed.allowed_origins = vec!["https://console.example.com".to_owned()];
        assert!(inspect_upgrade(&uri, &headers, direct, &allowed).is_ok());
    }

    #[test]
    fn malformed_forwarded_chain_is_rejected() {
        let uri: http::Uri = "/ws/id".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "X-Forwarded-For",
            "198.51.100.7, not-an-ip".parse().unwrap(),
        );
        let direct: SocketAddr = "127.0.0.1:32100".parse().unwrap();
        assert!(inspect_upgrade(&uri, &headers, direct, &config()).is_err());
    }

    #[test]
    fn non_signal_path_is_rejected() {
        let uri: http::Uri = "/ws/relay".parse().unwrap();
        let direct: SocketAddr = "127.0.0.1:32100".parse().unwrap();
        assert!(inspect_upgrade(&uri, &http::HeaderMap::new(), direct, &config()).is_err());
    }
}
