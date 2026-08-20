use base64::{encode_config, URL_SAFE_NO_PAD};
use hbb_common::{
    bytes::Bytes,
    futures_util::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::{
        punch_hole_response, register_pk_response, rendezvous_message, NatType, PunchHole,
        PunchHoleRequest, RegisterPeer, RegisterPk, RelayResponse, RendezvousMessage, RequestRelay,
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
use sodiumoxide::crypto::sign;
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        ServerConfig,
    },
    TlsAcceptor,
};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tungstenite::client::IntoClientRequest;
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
            sodiumoxide::init().unwrap();
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
            fs::create_dir_all(hbbs_dir.join("auth")).unwrap();
            let (auth_public, auth_secret) = sign::gen_keypair();
            fs::write(
                hbbs_dir.join("auth/jwks.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "keys": [{
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "use": "sig",
                        "alg": "EdDSA",
                        "kid": "persistent-wss-test-key",
                        "x": encode_config(auth_public.0, URL_SAFE_NO_PAD)
                    }]
                }))
                .unwrap(),
            )
            .unwrap();
            let active_token = connection_token(&auth_secret);

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

            assert_persistent_websocket_auth_denials(
                &mut websocket_a,
                "wss-peer-b001",
                &mut websocket_b,
                &server_key,
            )
            .await;

            assert_websocket_to_websocket(
                &mut websocket_a,
                "wss-peer-b001",
                &mut websocket_b,
                &relay,
                &server_key,
                &active_token,
            )
            .await;
            assert_websocket_request_relay(
                &mut websocket_a,
                "wss-peer-b001",
                &mut websocket_b,
                &relay,
                &active_token,
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
                &active_token,
            )
            .await;
            assert_native_to_websocket(
                hbbs_port,
                &mut websocket_b,
                "wss-peer-b001",
                &relay,
                &server_key,
                &active_token,
            )
            .await;
        });
}

#[test]
#[ignore = "release-only 1,000-connection load gate"]
fn hbbs_sustains_one_thousand_registered_idle_websockets() {
    Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let unused_relay_port = reserve_hbbr_ports(hbbs_port);
            let root = std::env::temp_dir().join(format!(
                "starry-websocket-load-{}-{hbbs_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();

            let (probe_port, ca_path, probe_count, probe_task) = start_wss_probe(&root).await;
            let hbbs_dir = root.join("hbbs");
            fs::create_dir_all(&hbbs_dir).unwrap();
            let relay = format!("localhost:{unused_relay_port}");
            let config_path = hbbs_dir.join("config.yaml");
            fs::write(&config_path, websocket_load_config(&relay, probe_port)).unwrap();
            let hbbs = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(hbbs_port.to_string())
                .arg(format!("--starry-config={}", config_path.display()))
                .env("SSL_CERT_FILE", &ca_path)
                .env("RUST_LOG", "error")
                .current_dir(&hbbs_dir)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = TestEnvironment {
                children: vec![hbbs],
                tasks: vec![probe_task],
                root,
            };

            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port + 2))).await;
            wait_for_wss_probe(&probe_count).await;

            let mut websockets = Vec::with_capacity(1_000);
            // Ramp up in bounded batches. The steady-state target is 1,000
            // idle sessions; an unbounded SYN/SQLite thundering herd measures
            // the host backlog instead of the session implementation.
            for batch_start in (0..1_000_u32).step_by(25) {
                let mut registrations = Vec::with_capacity(25);
                for index in batch_start..batch_start + 25 {
                    registrations.push(hbb_common::tokio::spawn(async move {
                        let id = format!("load-peer-{index:07}");
                        let word = index.to_be_bytes();
                        let uuid = word.repeat(4);
                        let public_key = word.repeat(8);
                        let effective_ip =
                            format!("10.200.{}.{}", (index / 250) % 4, (index % 250) + 1);
                        connect_registered_websocket_with_identity_and_ip(
                            hbbs_port,
                            &id,
                            &uuid,
                            &public_key,
                            Some(&effective_ip),
                        )
                        .await
                    }));
                }
                for registration in registrations {
                    websockets.push(registration.await.unwrap());
                }
            }
            assert_eq!(websockets.len(), 1_000);

            let mut probes = Vec::with_capacity(1_000);
            for (index, websocket) in websockets.into_iter().enumerate() {
                probes.push(hbb_common::tokio::spawn(async move {
                    let payload = (index as u64).to_be_bytes().to_vec();
                    verify_websocket_ping(websocket, payload).await
                }));
            }
            let mut verified = Vec::with_capacity(1_000);
            for probe in probes {
                verified.push(probe.await.unwrap());
            }
            assert_eq!(verified.len(), 1_000);
            sleep(Duration::from_secs(2)).await;

            // Re-register a canary subset without first closing the original
            // socket. The new generation must atomically replace the old one
            // and remain live under a bounded reconnect storm.
            let mut reconnects = Vec::with_capacity(100);
            for index in 0..100_u32 {
                reconnects.push(hbb_common::tokio::spawn(async move {
                    let id = format!("load-peer-{index:07}");
                    let word = index.to_be_bytes();
                    let uuid = word.repeat(4);
                    let public_key = word.repeat(8);
                    let effective_ip =
                        format!("10.200.{}.{}", (index / 250) % 4, (index % 250) + 1);
                    let websocket = connect_registered_websocket_with_identity_and_ip(
                        hbbs_port,
                        &id,
                        &uuid,
                        &public_key,
                        Some(&effective_ip),
                    )
                    .await;
                    verify_websocket_ping(
                        websocket,
                        (10_000_u64 + u64::from(index)).to_be_bytes().to_vec(),
                    )
                    .await
                }));
            }
            for reconnect in reconnects {
                verified.push(reconnect.await.unwrap());
            }
            assert_eq!(verified.len(), 1_100);

            let mut closures = Vec::with_capacity(1_100);
            for mut websocket in verified {
                closures.push(hbb_common::tokio::spawn(async move {
                    let _ = websocket.close(None).await;
                }));
            }
            for closure in closures {
                closure.await.unwrap();
            }
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
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key)),
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
        r#"version: 3
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
connection_auth:
  mode: enforce
  issuer: https://api.example.test
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  jwks:
    file: auth/jwks.json
"#
    )
}

fn websocket_load_config(relay: &str, probe_port: u16) -> String {
    format!(
        r#"version: 3
relay_servers:
  - {relay}
websocket_signal:
  enabled: true
  registration_timeout_ms: 15000
  keepalive_interval_ms: 30000
  idle_timeout_ms: 120000
  max_frame_bytes: 65536
  outbound_queue_capacity: 64
  max_sessions: 1200
  max_sessions_per_effective_ip: 1200
  registration_rate_per_minute: 5000
  trusted_proxies:
    - 127.0.0.1/32
    - ::1/128
  allowed_origins: []
  relay_health:
    interval_seconds: 60
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
    connect_registered_websocket_with_identity(port, id, &[identity_byte; 16], &[identity_byte; 32])
        .await
}

async fn connect_registered_websocket_with_identity(
    port: u16,
    id: &str,
    uuid: &[u8],
    public_key: &[u8],
) -> ClientWebSocket {
    connect_registered_websocket_with_identity_and_ip(port, id, uuid, public_key, None).await
}

async fn connect_registered_websocket_with_identity_and_ip(
    port: u16,
    id: &str,
    uuid: &[u8],
    public_key: &[u8],
    effective_ip: Option<&str>,
) -> ClientWebSocket {
    assert_eq!(uuid.len(), 16);
    assert_eq!(public_key.len(), 32);
    let url = format!("ws://127.0.0.1:{}/ws/id", port + 2);
    for _ in 0..100 {
        let mut request = url.clone().into_client_request().unwrap();
        if let Some(effective_ip) = effective_ip {
            request
                .headers_mut()
                .insert("X-Real-IP", effective_ip.parse().unwrap());
        }
        let Ok((mut websocket, _)) = connect_async(request).await else {
            sleep(Duration::from_millis(50)).await;
            continue;
        };

        let mut register_peer = RendezvousMessage::new();
        register_peer.set_register_peer(RegisterPeer {
            id: id.to_owned(),
            ..Default::default()
        });
        if websocket
            .send(Message::Binary(
                register_peer.write_to_bytes().unwrap().into(),
            ))
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
            uuid: Bytes::copy_from_slice(uuid),
            pk: Bytes::copy_from_slice(public_key),
            ..Default::default()
        });
        if websocket
            .send(Message::Binary(
                register_pk.write_to_bytes().unwrap().into(),
            ))
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

async fn verify_websocket_ping(
    mut websocket: ClientWebSocket,
    payload: Vec<u8>,
) -> ClientWebSocket {
    websocket
        .send(Message::Ping(payload.clone().into()))
        .await
        .unwrap();
    timeout(10_000, async {
        loop {
            match websocket.next().await.unwrap().unwrap() {
                Message::Pong(received) if received == payload => break,
                Message::Ping(received) => websocket.send(Message::Pong(received)).await.unwrap(),
                Message::Close(frame) => {
                    panic!("load connection closed before Pong: {frame:?}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("load connection did not receive Pong");
    websocket
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

async fn assert_persistent_websocket_auth_denials(
    controller: &mut ClientWebSocket,
    target_id: &str,
    target: &mut ClientWebSocket,
    server_key: &str,
) {
    send_punch_request_ws(controller, target_id, server_key, "").await;
    match receive_protocol(controller, 5_000).await.unwrap().union {
        Some(rendezvous_message::Union::PunchHoleResponse(response)) => {
            assert_eq!(
                response.failure.enum_value().ok(),
                Some(punch_hole_response::Failure::OFFLINE)
            );
            assert_eq!(response.other_failure, "connection authorization failed");
        }
        other => panic!("expected persistent WSS PunchHoleResponse denial, got {other:?}"),
    }
    assert_target_received_nothing(target, "denied persistent WSS PunchHoleRequest").await;

    send_request_relay_ws(controller, target_id, "relay.invalid.test:21117", "").await;
    match receive_protocol(controller, 5_000).await.unwrap().union {
        Some(rendezvous_message::Union::RelayResponse(response)) => {
            assert_eq!(response.refuse_reason, "connection authorization failed")
        }
        other => panic!("expected persistent WSS RelayResponse denial, got {other:?}"),
    }
    assert_target_received_nothing(target, "denied persistent WSS RequestRelay").await;
}

async fn assert_target_received_nothing(target: &mut ClientWebSocket, context: &str) {
    match receive_protocol(target, 250).await {
        Err(error) if error == "WebSocket protocol response timed out" => {}
        other => panic!("{context} reached or closed the target: {other:?}"),
    }
}

async fn assert_websocket_request_relay(
    controller: &mut ClientWebSocket,
    target_id: &str,
    target: &mut ClientWebSocket,
    relay: &str,
    token: &str,
) {
    send_request_relay_ws(controller, target_id, relay, token).await;
    let request = match receive_protocol(target, 5_000).await.unwrap().union {
        Some(rendezvous_message::Union::RequestRelay(request)) => request,
        other => panic!("expected persistent WSS RequestRelay, got {other:?}"),
    };
    assert_eq!(request.id, target_id);
    assert_eq!(request.relay_server, relay);
    assert!(!request.socket_addr.is_empty());
    let mut response = RendezvousMessage::new();
    response.set_relay_response(RelayResponse {
        socket_addr: request.socket_addr,
        uuid: request.uuid,
        relay_server: request.relay_server,
        ..Default::default()
    });
    target
        .send(Message::Binary(response.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    expect_relay_response(receive_protocol(controller, 5_000).await.unwrap(), relay);
}

async fn send_request_relay_ws(
    websocket: &mut ClientWebSocket,
    target_id: &str,
    relay: &str,
    token: &str,
) {
    let mut request = RendezvousMessage::new();
    request.set_request_relay(RequestRelay {
        id: target_id.to_owned(),
        uuid: "persistent-wss-auth".to_owned(),
        relay_server: relay.to_owned(),
        token: token.to_owned(),
        ..Default::default()
    });
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
}

async fn assert_websocket_to_websocket(
    controller: &mut ClientWebSocket,
    target_id: &str,
    target: &mut ClientWebSocket,
    relay: &str,
    server_key: &str,
    token: &str,
) {
    send_punch_request_ws(controller, target_id, server_key, token).await;
    let punch = expect_punch(receive_protocol(target, 5_000).await.unwrap(), relay);
    target
        .send(Message::Binary(
            relay_response(&punch).write_to_bytes().unwrap().into(),
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
    token: &str,
) {
    send_punch_request_ws(controller, target_id, server_key, token).await;
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
    token: &str,
) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut controller = FramedStream::from(stream, peer_addr);
    controller
        .send(&punch_request(target_id, server_key, token))
        .await
        .unwrap();
    let punch = expect_punch(receive_protocol(target, 5_000).await.unwrap(), relay);
    target
        .send(Message::Binary(
            relay_response(&punch).write_to_bytes().unwrap().into(),
        ))
        .await
        .unwrap();
    let response = controller.next_timeout(5_000).await.unwrap().unwrap();
    expect_relay_response(
        RendezvousMessage::parse_from_bytes(&response).unwrap(),
        relay,
    );
}

async fn send_punch_request_ws(
    websocket: &mut ClientWebSocket,
    target_id: &str,
    server_key: &str,
    token: &str,
) {
    websocket
        .send(Message::Binary(
            punch_request(target_id, server_key, token)
                .write_to_bytes()
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
}

fn punch_request(target_id: &str, server_key: &str, token: &str) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_punch_hole_request(PunchHoleRequest {
        id: target_id.to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        licence_key: server_key.to_owned(),
        token: token.to_owned(),
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

fn connection_token(secret: &sign::SecretKey) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let header = encode_config(
        serde_json::to_vec(&serde_json::json!({
            "alg": "EdDSA",
            "kid": "persistent-wss-test-key",
            "typ": "at+jwt"
        }))
        .unwrap(),
        URL_SAFE_NO_PAD,
    );
    let payload = encode_config(
        serde_json::to_vec(&serde_json::json!({
            "iss": "https://api.example.test",
            "aud": "rustdesk-connect",
            "token_use": "access",
            "scope": "connect:initiate",
            "sub": "1002",
            "user_id": 1_002,
            "auth_version": 1,
            "jti": "01941f29-7c30-7000-8000-000000001002",
            "iat": now.saturating_sub(1),
            "nbf": now.saturating_sub(1),
            "exp": now.saturating_add(60)
        }))
        .unwrap(),
        URL_SAFE_NO_PAD,
    );
    let signing_input = format!("{header}.{payload}");
    let signature = sign::sign_detached(signing_input.as_bytes(), secret);
    format!(
        "{signing_input}.{}",
        encode_config(signature.as_ref(), URL_SAFE_NO_PAD)
    )
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
