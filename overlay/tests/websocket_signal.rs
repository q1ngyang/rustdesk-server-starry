use hbb_common::{
    bytes::Bytes,
    futures_util::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::{
        register_pk_response, rendezvous_message, NatType, PunchHole, PunchHoleRequest,
        RegisterPeer, RegisterPk, RelayResponse, RendezvousMessage,
    },
    tcp::FramedStream,
    timeout,
    tokio::{
        net::{TcpListener, TcpStream},
        runtime::Builder,
        task::JoinHandle,
        time::{sleep, Duration},
    },
    udp::FramedSocket,
};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa,
    PKCS_ECDSA_P256_SHA256,
};
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio_rustls::{
    rustls::{Certificate as RustlsCertificate, PrivateKey, ServerConfig},
    TlsAcceptor,
};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

type ClientWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct TestEnvironment {
    children: Vec<Child>,
    tasks: Vec<JoinHandle<()>>,
    root: PathBuf,
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn hbbs_registers_websocket_peers_and_routes_both_transports() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let hbbs_port = reserve_hbbs_ports();
            let hbbr_port = reserve_hbbr_ports(hbbs_port);
            let root = std::env::temp_dir().join(format!(
                "starry-websocket-signal-{}-{hbbs_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();

            let (probe_port, ca_path, probe_count, probe_task) = start_wss_probe(&root).await;
            let hbbr_dir = root.join("hbbr");
            let hbbs_dir = root.join("hbbs");
            fs::create_dir_all(&hbbr_dir).unwrap();
            fs::create_dir_all(&hbbs_dir).unwrap();

            let hbbr = Command::new(env!("CARGO_BIN_EXE_hbbr"))
                .arg("--port")
                .arg(hbbr_port.to_string())
                .env("RUST_LOG", "warn")
                .current_dir(&hbbr_dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbr_port))).await;

            let relay = format!("localhost:{hbbr_port}");
            let config_path = hbbs_dir.join("config.yaml");
            fs::write(&config_path, websocket_config(&relay, probe_port)).unwrap();
            let hbbs = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(hbbs_port.to_string())
                .arg(format!("--starry-config={}", config_path.display()))
                .env("SSL_CERT_FILE", &ca_path)
                .env("RUST_LOG", "warn")
                .current_dir(&hbbs_dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = TestEnvironment {
                children: vec![hbbs, hbbr],
                tasks: vec![probe_task],
                root,
            };

            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port + 2))).await;
            wait_for_wss_probe(&probe_count).await;
            let server_key = wait_for_server_key(&hbbs_dir).await;

            let mut websocket_a =
                connect_registered_websocket(hbbs_port, "wss-peer-a001", 0x11).await;
            wait_for_empty_heartbeat(&mut websocket_a).await;
            let mut websocket_b =
                connect_registered_websocket(hbbs_port, "wss-peer-b001", 0x22).await;

            assert_websocket_to_websocket(
                &mut websocket_a,
                "wss-peer-b001",
                &mut websocket_b,
                &relay,
                &server_key,
            )
            .await;

            // Official HBBS refreshes the native Relay online list on a short
            // interval. Wait for that list before exercising Mixed selection.
            sleep(Duration::from_millis(3_500)).await;

            let mut native_peer = register_native_peer(hbbs_port, "native-peer001", 0x33).await;
            assert_websocket_to_native(
                hbbs_port,
                &mut websocket_a,
                "native-peer001",
                &mut native_peer,
                &relay,
                &server_key,
            )
            .await;
            assert_native_to_websocket(
                hbbs_port,
                &mut websocket_b,
                "wss-peer-b001",
                &relay,
                &server_key,
            )
            .await;
        });
}

async fn start_wss_probe(root: &Path) -> (u16, PathBuf, Arc<AtomicUsize>, JoinHandle<()>) {
    let mut ca_params = CertificateParams::new(Vec::new());
    ca_params.alg = &PKCS_ECDSA_P256_SHA256;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_name = DistinguishedName::new();
    ca_name.push(DnType::CommonName, "Starry integration test CA");
    ca_params.distinguished_name = ca_name;
    let ca = Certificate::from_params(ca_params).unwrap();

    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]);
    server_params.alg = &PKCS_ECDSA_P256_SHA256;
    let mut server_name = DistinguishedName::new();
    server_name.push(DnType::CommonName, "localhost");
    server_params.distinguished_name = server_name;
    let server = Certificate::from_params(server_params).unwrap();

    let ca_path = root.join("test-ca.pem");
    fs::write(&ca_path, ca.serialize_pem().unwrap()).unwrap();
    let certificate = server.serialize_der_with_signer(&ca).unwrap();
    let private_key = server.serialize_private_key_der();
    let tls = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            vec![RustlsCertificate(certificate)],
            PrivateKey(private_key),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let probe_count = Arc::new(AtomicUsize::new(0));
    let task_probe_count = probe_count.clone();

    let task = hbb_common::tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let probe_count = task_probe_count.clone();
            hbb_common::tokio::spawn(async move {
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        eprintln!("test /ws/relay TLS accept failed: {err}");
                        return;
                    }
                };
                let callback = |request: &http::Request<()>, response| {
                    if request.uri().path() == "/ws/relay" && request.uri().query().is_none() {
                        Ok(response)
                    } else {
                        Err(http::Response::builder()
                            .status(http::StatusCode::NOT_FOUND)
                            .body(Some("Not Found".to_owned()))
                            .unwrap())
                    }
                };
                let mut websocket =
                    match tokio_tungstenite::accept_hdr_async(stream, callback).await {
                        Ok(websocket) => websocket,
                        Err(err) => {
                            eprintln!("test /ws/relay Upgrade failed: {err}");
                            return;
                        }
                    };
                probe_count.fetch_add(1, Ordering::SeqCst);
                while let Some(message) = websocket.next().await {
                    match message {
                        Ok(Message::Ping(bytes)) => {
                            if websocket.send(Message::Pong(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });
    // The test uses a current-thread runtime. Schedule the accept loop once
    // before the external HBBS process can begin its immediate health probe.
    hbb_common::tokio::task::yield_now().await;
    (port, ca_path, probe_count, task)
}

fn websocket_config(relay: &str, probe_port: u16) -> String {
    format!(
        r#"version: 2
relay_servers:
  - {relay}
websocket_signal:
  enabled: true
  registration_timeout_ms: 3000
  keepalive_interval_ms: 1000
  idle_timeout_ms: 15000
  max_frame_bytes: 65536
  outbound_queue_capacity: 64
  max_sessions: 100
  max_sessions_per_effective_ip: 100
  registration_rate_per_minute: 100
  trusted_proxies:
    - 127.0.0.1/32
    - ::1/128
  allowed_origins: []
  relay_health:
    interval_seconds: 5
    timeout_ms: 2000
    success_threshold: 1
    failure_threshold: 1
    endpoints:
      - relay: {relay}
        url: wss://localhost:{probe_port}/ws/relay
"#
    )
}

async fn connect_registered_websocket(port: u16, id: &str, identity_byte: u8) -> ClientWebSocket {
    let url = format!("ws://127.0.0.1:{}/ws/id", port + 2);
    for _ in 0..100 {
        let Ok((mut websocket, _)) = connect_async(&url).await else {
            sleep(Duration::from_millis(50)).await;
            continue;
        };

        let mut register_peer = RendezvousMessage::new();
        register_peer.set_register_peer(RegisterPeer {
            id: id.to_owned(),
            ..Default::default()
        });
        if websocket
            .send(Message::Binary(register_peer.write_to_bytes().unwrap()))
            .await
            .is_err()
        {
            continue;
        }
        let Ok(peer_response) = receive_protocol(&mut websocket, 1_000).await else {
            continue;
        };
        match peer_response.union {
            Some(rendezvous_message::Union::RegisterPeerResponse(response))
                if response.request_pk => {}
            _ => continue,
        }

        let mut register_pk = RendezvousMessage::new();
        register_pk.set_register_pk(RegisterPk {
            id: id.to_owned(),
            uuid: Bytes::from(vec![identity_byte; 16]),
            pk: Bytes::from(vec![identity_byte; 32]),
            ..Default::default()
        });
        if websocket
            .send(Message::Binary(register_pk.write_to_bytes().unwrap()))
            .await
            .is_err()
        {
            continue;
        }
        if let Ok(response) = receive_protocol(&mut websocket, 1_000).await {
            if let Some(rendezvous_message::Union::RegisterPkResponse(response)) = response.union {
                if response.result.enum_value().ok() == Some(register_pk_response::Result::OK) {
                    assert!(response.keep_alive > 0);
                    return websocket;
                }
            }
        }
        let _ = websocket.close(None).await;
        sleep(Duration::from_millis(100)).await;
    }
    panic!("HBBS did not accept WebSocket registration for {id}");
}

async fn wait_for_empty_heartbeat(websocket: &mut ClientWebSocket) {
    timeout(4_000, async {
        loop {
            match websocket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) if bytes.is_empty() => return,
                Message::Ping(bytes) => websocket.send(Message::Pong(bytes)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .expect("no empty WebSocket heartbeat");
}

async fn receive_protocol(
    websocket: &mut ClientWebSocket,
    timeout_ms: u64,
) -> Result<RendezvousMessage, String> {
    timeout(timeout_ms, async {
        loop {
            let message = websocket
                .next()
                .await
                .ok_or_else(|| "WebSocket closed".to_owned())?
                .map_err(|err| err.to_string())?;
            match message {
                Message::Binary(bytes) if !bytes.is_empty() => {
                    return RendezvousMessage::parse_from_bytes(&bytes)
                        .map_err(|err| err.to_string())
                }
                Message::Ping(bytes) => websocket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|err| err.to_string())?,
                Message::Close(_) => return Err("WebSocket closed".to_owned()),
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "WebSocket protocol response timed out".to_owned())?
}

async fn assert_websocket_to_websocket(
    controller: &mut ClientWebSocket,
    target_id: &str,
    target: &mut ClientWebSocket,
    relay: &str,
    server_key: &str,
) {
    send_punch_request_ws(controller, target_id, server_key).await;
    let punch = expect_punch(receive_protocol(target, 5_000).await.unwrap(), relay);
    target
        .send(Message::Binary(
            relay_response(&punch).write_to_bytes().unwrap(),
        ))
        .await
        .unwrap();
    expect_relay_response(receive_protocol(controller, 5_000).await.unwrap(), relay);
}

async fn register_native_peer(port: u16, id: &str, identity_byte: u8) -> FramedSocket {
    let mut socket = FramedSocket::new("0.0.0.0:0").await.unwrap();
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let mut register_peer = RendezvousMessage::new();
    register_peer.set_register_peer(RegisterPeer {
        id: id.to_owned(),
        ..Default::default()
    });
    socket.send(&register_peer, server).await.unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    let response = RendezvousMessage::parse_from_bytes(&bytes).unwrap();
    assert!(matches!(
        response.union,
        Some(rendezvous_message::Union::RegisterPeerResponse(response)) if response.request_pk
    ));

    let mut register_pk = RendezvousMessage::new();
    register_pk.set_register_pk(RegisterPk {
        id: id.to_owned(),
        uuid: Bytes::from(vec![identity_byte; 16]),
        pk: Bytes::from(vec![identity_byte; 32]),
        ..Default::default()
    });
    socket.send(&register_pk, server).await.unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    let response = RendezvousMessage::parse_from_bytes(&bytes).unwrap();
    assert!(matches!(
        response.union,
        Some(rendezvous_message::Union::RegisterPkResponse(response))
            if response.result.enum_value().ok() == Some(register_pk_response::Result::OK)
    ));
    socket
}

async fn assert_websocket_to_native(
    port: u16,
    controller: &mut ClientWebSocket,
    target_id: &str,
    target: &mut FramedSocket,
    relay: &str,
    server_key: &str,
) {
    send_punch_request_ws(controller, target_id, server_key).await;
    let (bytes, _) = target.next_timeout(5_000).await.unwrap().unwrap();
    let punch = expect_punch(RendezvousMessage::parse_from_bytes(&bytes).unwrap(), relay);

    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut response_stream = FramedStream::from(stream, peer_addr);
    response_stream.send(&relay_response(&punch)).await.unwrap();
    expect_relay_response(receive_protocol(controller, 5_000).await.unwrap(), relay);
}

async fn assert_native_to_websocket(
    port: u16,
    target: &mut ClientWebSocket,
    target_id: &str,
    relay: &str,
    server_key: &str,
) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut controller = FramedStream::from(stream, peer_addr);
    controller
        .send(&punch_request(target_id, server_key))
        .await
        .unwrap();
    let punch = expect_punch(receive_protocol(target, 5_000).await.unwrap(), relay);
    target
        .send(Message::Binary(
            relay_response(&punch).write_to_bytes().unwrap(),
        ))
        .await
        .unwrap();
    let response = controller.next_timeout(5_000).await.unwrap().unwrap();
    expect_relay_response(
        RendezvousMessage::parse_from_bytes(&response).unwrap(),
        relay,
    );
}

async fn send_punch_request_ws(websocket: &mut ClientWebSocket, target_id: &str, server_key: &str) {
    websocket
        .send(Message::Binary(
            punch_request(target_id, server_key)
                .write_to_bytes()
                .unwrap(),
        ))
        .await
        .unwrap();
}

fn punch_request(target_id: &str, server_key: &str) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_punch_hole_request(PunchHoleRequest {
        id: target_id.to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        licence_key: server_key.to_owned(),
        ..Default::default()
    });
    message
}

fn expect_punch(message: RendezvousMessage, relay: &str) -> PunchHole {
    match message.union {
        Some(rendezvous_message::Union::PunchHole(punch)) => {
            assert_eq!(punch.relay_server, relay);
            assert_eq!(punch.nat_type.enum_value().ok(), Some(NatType::SYMMETRIC));
            punch
        }
        other => panic!("expected PunchHole, got {other:?}"),
    }
}

fn relay_response(punch: &PunchHole) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_relay_response(RelayResponse {
        socket_addr: punch.socket_addr.clone(),
        relay_server: punch.relay_server.clone(),
        ..Default::default()
    });
    message
}

fn expect_relay_response(message: RendezvousMessage, relay: &str) {
    match message.union {
        Some(rendezvous_message::Union::RelayResponse(response)) => {
            assert_eq!(response.relay_server, relay)
        }
        other => panic!("expected RelayResponse, got {other:?}"),
    }
}

fn reserve_hbbs_ports() -> u16 {
    for port in 30_001..40_000u16 {
        let Ok(nat) = StdTcpListener::bind(("127.0.0.1", port - 1)) else {
            continue;
        };
        let Ok(tcp) = StdTcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        let Ok(udp) = StdUdpSocket::bind(("127.0.0.1", port)) else {
            continue;
        };
        let Ok(websocket) = StdTcpListener::bind(("127.0.0.1", port + 2)) else {
            continue;
        };
        drop(websocket);
        drop(udp);
        drop(tcp);
        drop(nat);
        return port;
    }
    panic!("no free HBBS port set");
}

fn reserve_hbbr_ports(hbbs_port: u16) -> u16 {
    for port in 40_000..60_000u16 {
        if port == hbbs_port || port == hbbs_port + 2 {
            continue;
        }
        let Ok(native) = StdTcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        let Some(websocket_port) = port.checked_add(2) else {
            continue;
        };
        let Ok(websocket) = StdTcpListener::bind(("127.0.0.1", websocket_port)) else {
            continue;
        };
        drop(websocket);
        drop(native);
        return port;
    }
    panic!("no free HBBR port pair");
}

async fn wait_until_listening(addr: SocketAddr) {
    for _ in 0..200 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("server did not listen on {addr}");
}

async fn wait_for_server_key(directory: &Path) -> String {
    let path = directory.join("id_ed25519.pub");
    for _ in 0..200 {
        if let Ok(key) = fs::read_to_string(&path) {
            let key = key.trim();
            if !key.is_empty() {
                return key.to_owned();
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("HBBS did not create its temporary public key");
}

async fn wait_for_wss_probe(probe_count: &AtomicUsize) {
    for _ in 0..750 {
        if probe_count.load(Ordering::SeqCst) > 0 {
            // Let HBBS record the successful probe after its close frame.
            sleep(Duration::from_millis(100)).await;
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("HBBS did not complete a certificate-valid /ws/relay probe");
}
