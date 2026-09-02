use hbb_common::{
    bytes::Bytes,
    futures_util::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::{rendezvous_message, RelayProbeRequest, RendezvousMessage, RequestRelay},
    tcp::FramedStream,
    timeout,
    tokio::{
        io::copy_bidirectional,
        net::{TcpListener as TokioTcpListener, TcpStream},
        runtime::Builder,
        task::JoinHandle,
        time::{sleep, Duration},
    },
};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa,
    PKCS_ECDSA_P256_SHA256,
};
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::auth;
use std::{
    fs,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        ClientConfig, RootCertStore, ServerConfig,
    },
    TlsAcceptor,
};
use tokio_tungstenite::{client_async_tls_with_config, Connector};
use tungstenite::{client::IntoClientRequest, Message};

struct RelayProcess {
    child: Child,
    state_dir: PathBuf,
}

impl Drop for RelayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.state_dir);
    }
}

#[test]
fn official_hbbr_bridges_websocket_and_native_streams() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let port = reserve_port_pair();
        let state_dir =
            std::env::temp_dir().join(format!("starry-mixed-relay-{}-{port}", std::process::id()));
        fs::create_dir_all(&state_dir).unwrap();
        let telemetry_secret_path = state_dir.join("relay-telemetry.secret");
        let draining_file = state_dir.join("relay.draining");
        let telemetry_secret = b"starry-mixed-relay-telemetry-secret-at-least-32-bytes";
        fs::write(&telemetry_secret_path, telemetry_secret).unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_hbbr"))
            .arg("--port")
            .arg(port.to_string())
            .env("STARRY_RELAY_MAX_SESSIONS", "1")
            .env("STARRY_RELAY_PROBE_PER_IP_PER_MINUTE", "4")
            .env("STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE", "100")
            .env("STARRY_RELAY_TELEMETRY_SECRET_FILE", &telemetry_secret_path)
            .env("STARRY_RELAY_DRAINING_FILE", &draining_file)
            .env("TOTAL_BANDWIDTH", "64")
            .current_dir(&state_dir)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let _relay = RelayProcess { child, state_dir };
        wait_until_listening(port).await;
        wait_until_listening(port + 2).await;

        assert_active_probe_and_public_headers(port).await;
        assert_telemetry_requires_authentication(port).await;
        let telemetry = authenticated_telemetry(port, telemetry_secret).await;
        assert_eq!(telemetry["telemetry_schema"], 2);
        assert_eq!(telemetry["fast_media"]["protocol"], 1);
        assert_eq!(telemetry["fast_media"]["enabled"], false);
        assert_eq!(telemetry["fast_media"]["healthy"], false);
        assert_eq!(telemetry["fast_media"]["udp_port"], 0);
        assert_eq!(telemetry["capacity_sessions"], 1);
        assert_eq!(telemetry["active_sessions"], 0);
        assert_eq!(telemetry["pending_pairs"], 0);
        assert_eq!(telemetry["bandwidth_ema_alpha_basis_points"], 2500);
        assert_eq!(telemetry["admission_open"], true);
        assert_native_active_probe(port).await;
        let (wss_port, wss_connector, _wss_proxy) = start_wss_proxy(port + 2).await;
        assert_wss_active_probe(wss_port, wss_connector).await;
        assert_probe_classification_counters(port, telemetry_secret).await;
        assert_ws_probe_is_rate_limited(port).await;
        assert_admission_lifecycle(port, telemetry_secret, &draining_file).await;
        run_pair(port, "starry-mixed-ws-first", true).await;
        run_pair(port, "starry-mixed-native-first", false).await;
    });
}

async fn assert_active_probe_and_public_headers(port: u16) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, response) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    assert!(header("x-starry-version")
        .as_deref()
        .is_some_and(|value| value.ends_with("-patch-v1.3.1")));
    assert_eq!(
        header("x-starry-relay-probe-protocol").as_deref(),
        Some("1")
    );
    assert_eq!(header("x-starry-relay-load-protocol").as_deref(), Some("1"));
    for private_header in [
        "x-starry-telemetry",
        "x-starry-telemetry-auth",
        "x-starry-load-bps",
        "x-starry-active-sessions",
        "x-starry-capacity-sessions",
        "x-starry-bandwidth-bps",
        "x-starry-capacity-bandwidth-bps",
    ] {
        assert_eq!(
            header(private_header),
            None,
            "public load leak: {private_header}"
        );
    }

    let nonce = vec![0x73; 16];
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version: 1,
        nonce: nonce.clone().into(),
        ..Default::default()
    });
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    let payload = timeout(5_000, websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data();
    let response = RendezvousMessage::parse_from_bytes(&payload).unwrap();
    let Some(rendezvous_message::Union::RelayProbeResponse(response)) = response.union else {
        panic!("HBBR did not return RelayProbeResponse");
    };
    assert_eq!(response.protocol_version, 1);
    assert_eq!(response.nonce.as_ref(), nonce.as_slice());
    assert!(response.load.is_none());
    assert!(response.starry_version.ends_with("-patch-v1.3.1"));
    assert_eq!(response.relay_probe_protocol, 1);
    assert_eq!(response.relay_load_protocol, 1);
}

async fn assert_telemetry_requires_authentication(port: u16) {
    let error =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/ws/telemetry", port + 2))
            .await
            .unwrap_err();
    let tungstenite::Error::Http(response) = error else {
        panic!("unexpected unauthenticated telemetry error: {error}");
    };
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    for name in ["x-starry-telemetry", "x-starry-telemetry-auth"] {
        assert!(response.headers().get(name).is_none());
    }
}

async fn authenticated_telemetry(port: u16, secret: &[u8]) -> serde_json::Value {
    sodiumoxide::init().unwrap();
    let key = auth::Key::from_slice(&Sha256::digest(secret)).unwrap();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let canonical = format!("starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry");
    let signature = hex(auth::authenticate(canonical.as_bytes(), &key).as_ref());
    let mut request = format!("ws://127.0.0.1:{}/ws/telemetry", port + 2)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "x-starry-telemetry-timestamp",
        http::HeaderValue::from_str(&timestamp.to_string()).unwrap(),
    );
    request.headers_mut().insert(
        "x-starry-telemetry-nonce",
        http::HeaderValue::from_str(&nonce).unwrap(),
    );
    request.headers_mut().insert(
        "x-starry-telemetry-auth",
        http::HeaderValue::from_str(&signature).unwrap(),
    );
    let (mut websocket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    let payload = response
        .headers()
        .get("x-starry-telemetry")
        .unwrap()
        .to_str()
        .unwrap();
    let response_signature = response
        .headers()
        .get("x-starry-telemetry-auth")
        .unwrap()
        .to_str()
        .unwrap();
    let response_canonical = format!("starry-telemetry-response-v1\n{nonce}\n{payload}");
    assert!(verify_hex_hmac(
        response_signature,
        response_canonical.as_bytes(),
        &key
    ));
    let decoded = base64::decode_config(payload, base64::URL_SAFE_NO_PAD).unwrap();
    let _ = websocket.close(None).await;
    serde_json::from_slice(&decoded).unwrap()
}

async fn assert_ws_probe_is_rate_limited(port: u16) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version: 1,
        nonce: vec![0x72; 16].into(),
        ..Default::default()
    });
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    let result = timeout(5_000, websocket.next()).await.unwrap();
    assert!(result.is_none() || result.is_some_and(|message| message.is_err()));
}

async fn assert_probe_classification_counters(port: u16, secret: &[u8]) {
    send_rejected_probe(port, 1, vec![0x6d; 15]).await;
    send_rejected_probe(port, 2, vec![0x75; 16]).await;
    let telemetry = authenticated_telemetry(port, secret).await;
    assert!(telemetry["probe_malformed"].as_u64().unwrap() >= 1);
    assert!(telemetry["probe_unsupported"].as_u64().unwrap() >= 1);
    assert!(telemetry["probe_successful"].as_u64().unwrap() >= 3);
}

async fn send_rejected_probe(port: u16, protocol_version: u32, nonce: Vec<u8>) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version,
        nonce: nonce.into(),
        ..Default::default()
    });
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    let result = timeout(5_000, websocket.next()).await.unwrap();
    assert!(result.is_none() || result.is_some_and(|message| message.is_err()));
}

async fn assert_admission_lifecycle(port: u16, secret: &[u8], draining_file: &PathBuf) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let request = relay_request("starry-admission-lifecycle");
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;
    let pending = authenticated_telemetry(port, secret).await;
    assert_eq!(pending["pending_pairs"], 1);
    assert_eq!(pending["active_sessions"], 0);
    assert!(pending["probe_rate_limited"].as_u64().unwrap() >= 1);
    assert!(pending["telemetry_auth_failures"].as_u64().unwrap() >= 1);

    let native_ip = local_ip_address::local_ip().expect("no local IP for native relay leg");
    let stream = TcpStream::connect((native_ip, port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut native = FramedStream::from(stream, peer_addr);
    native.send(&request).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    let active = authenticated_telemetry(port, secret).await;
    assert_eq!(active["pending_pairs"], 0);
    assert_eq!(active["active_sessions"], 1);
    assert_eq!(active["admission_open"], false);

    let (mut rejected, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    rejected
        .send(Message::Binary(
            relay_request("starry-capacity-rejected")
                .write_to_bytes()
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    let rejected_result = timeout(5_000, rejected.next()).await.unwrap();
    assert!(rejected_result.is_none() || rejected_result.is_some_and(|message| message.is_err()));
    let capacity = authenticated_telemetry(port, secret).await;
    assert!(capacity["admission_rejections"].as_u64().unwrap() >= 1);

    fs::write(draining_file, b"drain").unwrap();
    let drain = authenticated_telemetry(port, secret).await;
    assert_eq!(drain["draining"], true);
    assert_eq!(drain["active_sessions"], 1);
    native
        .send_bytes(Bytes::from_static(b"existing-session-survives-drain"))
        .await
        .unwrap();
    let forwarded = timeout(5_000, websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        forwarded.into_data().as_ref(),
        b"existing-session-survives-drain"
    );
    let _ = websocket.close(None).await;
    drop(native);
    sleep(Duration::from_millis(150)).await;

    let (mut draining_rejected, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    draining_rejected
        .send(Message::Binary(
            relay_request("starry-draining-rejected")
                .write_to_bytes()
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    let result = timeout(5_000, draining_rejected.next()).await.unwrap();
    assert!(result.is_none() || result.is_some_and(|message| message.is_err()));
    let drained = authenticated_telemetry(port, secret).await;
    assert_eq!(drained["active_sessions"], 0);
    assert_eq!(drained["pending_pairs"], 0);
    assert!(drained["admission_rejections"].as_u64().unwrap() >= 2);
    fs::remove_file(draining_file).unwrap();
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verify_hex_hmac(signature: &str, message: &[u8], key: &auth::Key) -> bool {
    if signature.len() != auth::TAGBYTES * 2 {
        return false;
    }
    let mut decoded = vec![0_u8; auth::TAGBYTES];
    for (index, output) in decoded.iter_mut().enumerate() {
        let Ok(value) = u8::from_str_radix(&signature[index * 2..index * 2 + 2], 16) else {
            return false;
        };
        *output = value;
    }
    auth::Tag::from_slice(&decoded)
        .map(|tag| auth::verify(&tag, message, key))
        .unwrap_or(false)
}

async fn assert_native_active_probe(port: u16) {
    let native_ip = local_ip_address::local_ip().expect("no local IP for native Relay probe");
    assert!(!native_ip.is_loopback());
    let stream = TcpStream::connect((native_ip, port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut native = FramedStream::from(stream, peer_addr);
    let nonce = vec![0x4e; 16];
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version: 1,
        nonce: nonce.clone().into(),
        ..Default::default()
    });
    native.send(&request).await.unwrap();
    let payload = native.next_timeout(5_000).await.unwrap().unwrap();
    let response = RendezvousMessage::parse_from_bytes(&payload).unwrap();
    let Some(rendezvous_message::Union::RelayProbeResponse(response)) = response.union else {
        panic!("native HBBR did not return RelayProbeResponse");
    };
    assert_eq!(response.nonce.as_ref(), nonce.as_slice());
    assert_eq!(response.relay_probe_protocol, 1);
    assert_eq!(response.relay_load_protocol, 1);
    assert!(response.load.is_none());
}

async fn assert_wss_active_probe(port: u16, connector: Connector) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let ws_url = format!("wss://localhost:{port}/ws/relay");
    let (mut websocket, handshake) =
        client_async_tls_with_config(ws_url, stream, None, Some(connector))
            .await
            .unwrap();
    assert_eq!(
        handshake
            .headers()
            .get("x-starry-relay-probe-protocol")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    assert_eq!(
        handshake
            .headers()
            .get("x-starry-relay-load-protocol")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );

    let nonce = vec![0x57; 16];
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version: 1,
        nonce: nonce.clone().into(),
        ..Default::default()
    });
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    let payload = timeout(5_000, websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data();
    let response = RendezvousMessage::parse_from_bytes(&payload).unwrap();
    let Some(rendezvous_message::Union::RelayProbeResponse(response)) = response.union else {
        panic!("WSS HBBR did not return RelayProbeResponse");
    };
    assert_eq!(response.nonce.as_ref(), nonce.as_slice());
    assert_eq!(response.relay_probe_protocol, 1);
    assert_eq!(response.relay_load_protocol, 1);
    assert!(response.load.is_none());
}

async fn start_wss_proxy(upstream_port: u16) -> (u16, Connector, JoinHandle<()>) {
    let mut ca_params = CertificateParams::new(Vec::new());
    ca_params.alg = &PKCS_ECDSA_P256_SHA256;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_name = DistinguishedName::new();
    ca_name.push(DnType::CommonName, "Starry mixed relay test CA");
    ca_params.distinguished_name = ca_name;
    let ca = Certificate::from_params(ca_params).unwrap();

    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]);
    server_params.alg = &PKCS_ECDSA_P256_SHA256;
    let mut server_name = DistinguishedName::new();
    server_name.push(DnType::CommonName, "localhost");
    server_params.distinguished_name = server_name;
    let server = Certificate::from_params(server_params).unwrap();

    let ca_der = ca.serialize_der().unwrap();
    let server_der = server.serialize_der_with_signer(&ca).unwrap();
    let private_key = server.serialize_private_key_der();
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(server_der)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key)),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der)).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = Connector::Rustls(Arc::new(client_config));

    let listener = TokioTcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = hbb_common::tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            hbb_common::tokio::spawn(async move {
                let Ok(mut downstream) = acceptor.accept(stream).await else {
                    return;
                };
                let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", upstream_port)).await
                else {
                    return;
                };
                let _ = copy_bidirectional(&mut downstream, &mut upstream).await;
            });
        }
    });
    (port, connector, task)
}

async fn run_pair(port: u16, uuid: &str, websocket_first: bool) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, response) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let expected_version = format!(
        "{}-patch-v{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../PATCH_VERSION").trim()
    );
    assert_eq!(
        response
            .headers()
            .get("x-starry-version")
            .and_then(|value| value.to_str().ok()),
        Some(expected_version.as_str())
    );
    // Official HBBR reserves non-WebSocket loopback connections for its local
    // management command channel, so the native relay leg must use a
    // non-loopback local address even though the test server is on this host.
    let native_ip = local_ip_address::local_ip().expect("no local IP for native relay leg");
    assert!(
        !native_ip.is_loopback(),
        "native relay leg requires a non-loopback local IP"
    );
    let stream = TcpStream::connect((native_ip, port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut native = FramedStream::from(stream, peer_addr);

    let request = relay_request(uuid);
    if websocket_first {
        websocket
            .send(Message::Binary(request.write_to_bytes().unwrap().into()))
            .await
            .unwrap();
        sleep(Duration::from_millis(100)).await;
        native.send(&request).await.unwrap();
    } else {
        native.send(&request).await.unwrap();
        sleep(Duration::from_millis(100)).await;
        websocket
            .send(Message::Binary(request.write_to_bytes().unwrap().into()))
            .await
            .unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    let native_payload = Bytes::from_static(b"native-to-websocket");
    native.send_bytes(native_payload.clone()).await.unwrap();
    let ws_message = timeout(5_000, websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ws_message.into_data(), native_payload.as_ref());

    let websocket_payload = b"websocket-to-native";
    websocket
        .send(Message::Binary(websocket_payload.to_vec().into()))
        .await
        .unwrap();
    let native_message = native.next_timeout(5_000).await.unwrap().unwrap();
    assert_eq!(native_message.as_ref(), websocket_payload);

    let _ = websocket.close(None).await;
}

fn relay_request(uuid: &str) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_request_relay(RequestRelay {
        uuid: uuid.to_owned(),
        ..Default::default()
    });
    message
}

fn reserve_port_pair() -> u16 {
    for port in 30_000..60_000u16 {
        let Ok(native) = TcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        let Some(websocket_port) = port.checked_add(2) else {
            continue;
        };
        let Ok(websocket) = TcpListener::bind(("127.0.0.1", websocket_port)) else {
            continue;
        };
        drop(websocket);
        drop(native);
        return port;
    }
    panic!("no free relay port pair");
}

async fn wait_until_listening(port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    for _ in 0..100 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("relay did not listen on {addr}");
}
