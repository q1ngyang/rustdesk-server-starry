use base64::{encode_config, URL_SAFE_NO_PAD};
use hbb_common::{
    bytes::Bytes,
    bytes_codec::BytesCodec,
    futures_util::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::{
        punch_hole_response, register_pk_response, rendezvous_message, KeyExchange,
        PunchHoleRequest, RegisterPeer, RegisterPk, RelayResponse, RendezvousMessage, RequestRelay,
    },
    timeout,
    tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpStream, UdpSocket},
        runtime::Builder,
        time::{sleep, Duration},
    },
    tokio_util::codec::Framed,
    udp::FramedSocket,
};
use serde_json::{json, Value};
use sodiumoxide::crypto::{box_, secretbox, sign};
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use tokio_tungstenite::connect_async;
use tungstenite::Message;

const LOCAL_AUTH_TOKEN: &str = "localControlTokenForAuthTransportTest01";

struct ServerProcess {
    child: Child,
    state_dir: PathBuf,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.state_dir);
    }
}

#[test]
fn jwt_enforcement_covers_native_secure_tcp_websocket_and_udp_stays_unsupported() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let port = reserve_hbbs_ports();
            let state_dir = std::env::temp_dir().join(format!(
                "starry-connection-auth-{}-{port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&state_dir);
            fs::create_dir_all(state_dir.join("auth")).unwrap();
            let (public, secret) = sign::gen_keypair();
            fs::write(
                state_dir.join("auth/jwks.json"),
                serde_json::to_vec_pretty(&json!({
                    "keys": [{
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "use": "sig",
                        "alg": "EdDSA",
                        "kid": "transport-test-key",
                        "x": encode_config(public.0, URL_SAFE_NO_PAD)
                    }]
                }))
                .unwrap(),
            )
            .unwrap();
            let config_path = state_dir.join("config.yaml");
            fs::write(&config_path, auth_config()).unwrap();
            let local_control_token_path = state_dir.join("local-control.token");
            fs::write(&local_control_token_path, format!("{LOCAL_AUTH_TOKEN}\n")).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&local_control_token_path, fs::Permissions::from_mode(0o600))
                .unwrap();
            let child = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(port.to_string())
                .arg(format!("--starry-config={}", config_path.display()))
                .env("RUST_LOG", "warn")
                .env("STARRY_LOCAL_CONTROL_TOKEN_FILE", &local_control_token_path)
                .current_dir(&state_dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _server = ServerProcess { child, state_dir };
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], port))).await;
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], port + 2))).await;

            let active = token(&secret, "rustdesk-connect");
            let wrong_audience = token(&secret, "some-other-service");
            let target_id = "auth-target-exists";
            let mut target = register_native_peer(port, target_id).await;

            let expected_punch_denial = native_exchange(port, punch("")).await;
            assert_punch_denied(expected_punch_denial.clone());
            let expected_relay_denial = native_exchange(port, relay("")).await;
            assert_relay_denied(expected_relay_denial.clone());

            for denial in [
                native_exchange(port, punch_to(target_id, "")).await,
                secure_exchange(port, punch_to(target_id, "")).await,
                websocket_exchange(port, punch_to(target_id, "")).await,
            ] {
                assert_eq!(
                    denial.write_to_bytes().unwrap(),
                    expected_punch_denial.write_to_bytes().unwrap(),
                    "authentication denial must not reveal target existence or transport"
                );
                assert!(
                    target.next_timeout(200).await.is_none(),
                    "denied PunchHoleRequest reached the registered target"
                );
            }
            for denial in [
                native_exchange(port, relay_to(target_id, "")).await,
                secure_exchange(port, relay_to(target_id, "")).await,
                websocket_exchange(port, relay_to(target_id, "")).await,
            ] {
                assert_eq!(
                    denial.write_to_bytes().unwrap(),
                    expected_relay_denial.write_to_bytes().unwrap(),
                    "authentication denial must not reveal target existence or transport"
                );
                assert!(
                    target.next_timeout(200).await.is_none(),
                    "denied RequestRelay reached the registered target"
                );
            }
            assert_punch_denied(native_exchange(port, punch(&wrong_audience)).await);
            assert_normal_protocol_result(native_exchange(port, punch(&active)).await);

            assert_normal_protocol_result(secure_exchange(port, punch(&active)).await);
            assert_normal_protocol_result(websocket_exchange(port, punch(&active)).await);
            native_request_relay_is_not_denied(port, relay(&active)).await;

            let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            udp.send_to(
                &punch(&active).write_to_bytes().unwrap(),
                SocketAddr::from(([127, 0, 0, 1], port)),
            )
            .await
            .unwrap();
            let mut response = [0_u8; 2048];
            assert!(timeout(300, udp.recv_from(&mut response)).await.is_err());

            fs::write(&config_path, auth_config_audit()).unwrap();
            let reload = local_request(
                SocketAddr::from(([127, 0, 0, 1], port - 1)),
                "audit-reload",
                "runtime.reload",
            )
            .await;
            assert_eq!(reload["ok"], true, "audit reload failed: {reload}");
            assert_normal_protocol_result(native_exchange(port, punch("")).await);
            assert_normal_protocol_result(secure_exchange(port, punch("")).await);
            assert_normal_protocol_result(websocket_exchange(port, punch("")).await);
            let status = local_request(
                SocketAddr::from(([127, 0, 0, 1], port - 1)),
                "audit-status",
                "status",
            )
            .await;
            assert_eq!(status["result"]["auth"]["effective_mode"], "audit");
            assert!(
                status["result"]["auth"]["metrics"]["audit_would_deny"]
                    .as_u64()
                    .unwrap()
                    >= 3
            );
        });
}

fn auth_config() -> &'static str {
    r#"version: 3
relay_servers:
  - relay.example.test:21117
secure_tcp:
  mode: auto
websocket_signal:
  enabled: true
  relay_health:
    endpoints:
      - relay: relay.example.test:21117
        url: wss://localhost:9/ws/relay
connection_auth:
  mode: enforce
  issuer: https://api.example.test
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  jwks:
    file: auth/jwks.json
"#
}

fn auth_config_audit() -> &'static str {
    r#"version: 3
relay_servers:
  - relay.example.test:21117
secure_tcp:
  mode: auto
websocket_signal:
  enabled: true
  relay_health:
    endpoints:
      - relay: relay.example.test:21117
        url: wss://localhost:9/ws/relay
connection_auth:
  mode: audit
  issuer: https://api.example.test
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  jwks:
    file: auth/jwks.json
"#
}

fn token(secret: &sign::SecretKey, audience: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let header = encode_config(
        serde_json::to_vec(&json!({
            "alg": "EdDSA",
            "kid": "transport-test-key",
            "typ": "at+jwt"
        }))
        .unwrap(),
        URL_SAFE_NO_PAD,
    );
    let payload = encode_config(
        serde_json::to_vec(&json!({
            "iss": "https://api.example.test",
            "aud": audience,
            "token_use": "access",
            "scope": "connect:initiate",
            "sub": "1001",
            "user_id": 1_001,
            "auth_version": 1,
            "jti": "01941f29-7c30-7000-8000-000000001001",
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

fn punch(token: &str) -> RendezvousMessage {
    punch_to("target-does-not-exist", token)
}

fn punch_to(id: &str, token: &str) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_punch_hole_request(PunchHoleRequest {
        id: id.to_owned(),
        token: token.to_owned(),
        ..Default::default()
    });
    message
}

fn relay(token: &str) -> RendezvousMessage {
    relay_to("target-does-not-exist", token)
}

fn relay_to(id: &str, token: &str) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_request_relay(RequestRelay {
        id: id.to_owned(),
        uuid: "transport-auth-test".to_owned(),
        token: token.to_owned(),
        ..Default::default()
    });
    message
}

async fn register_native_peer(port: u16, id: &str) -> FramedSocket {
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
        uuid: Bytes::from(vec![0x44; 16]),
        pk: Bytes::from(vec![0x44; 32]),
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

fn assert_punch_denied(message: RendezvousMessage) {
    let Some(rendezvous_message::Union::PunchHoleResponse(response)) = message.union else {
        panic!("expected PunchHoleResponse denial");
    };
    assert_eq!(
        response.failure.enum_value().ok(),
        Some(punch_hole_response::Failure::OFFLINE)
    );
    assert_eq!(response.other_failure, "connection authorization failed");
}

fn assert_relay_denied(message: RendezvousMessage) {
    let Some(rendezvous_message::Union::RelayResponse(RelayResponse { refuse_reason, .. })) =
        message.union
    else {
        panic!("expected RelayResponse denial");
    };
    assert_eq!(refuse_reason, "connection authorization failed");
}

fn assert_normal_protocol_result(message: RendezvousMessage) {
    let Some(rendezvous_message::Union::PunchHoleResponse(response)) = message.union else {
        panic!("valid authentication did not reach normal protocol handling");
    };
    assert_eq!(
        response.failure.enum_value().ok(),
        Some(punch_hole_response::Failure::LICENSE_MISMATCH)
    );
    assert!(response.other_failure.is_empty());
}

async fn native_request_relay_is_not_denied(port: u16, request: RendezvousMessage) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut framed = Framed::new(stream, BytesCodec::new());
    let offer = framed.next().await.unwrap().unwrap();
    let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
    assert!(matches!(
        offer.union,
        Some(rendezvous_message::Union::KeyExchange(_))
    ));
    framed
        .send(Bytes::from(request.write_to_bytes().unwrap()))
        .await
        .unwrap();

    if let Ok(Some(Ok(response))) = timeout(500, framed.next()).await {
        let response = RendezvousMessage::parse_from_bytes(&response).unwrap();
        if let Some(rendezvous_message::Union::RelayResponse(response)) = response.union {
            assert_ne!(response.refuse_reason, "connection authorization failed");
        }
    }
}

async fn local_request(address: SocketAddr, request_id: &str, method: &str) -> Value {
    const MAGIC: &[u8] = b"STARRYCTL/1\n";
    const MAX_FRAME_BYTES: usize = 1024 * 1024;

    let payload = serde_json::to_vec(&json!({
        "request_id": request_id,
        "method": method,
        "auth_token": LOCAL_AUTH_TOKEN,
        "params": {}
    }))
    .unwrap();
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(MAGIC).await.unwrap();
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&payload).await.unwrap();

    let mut magic = [0_u8; MAGIC.len()];
    timeout(5_000, stream.read_exact(&mut magic))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&magic, MAGIC);
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await.unwrap();
    let length = u32::from_be_bytes(length) as usize;
    assert!(length <= MAX_FRAME_BYTES);
    let mut response = vec![0_u8; length];
    stream.read_exact(&mut response).await.unwrap();
    serde_json::from_slice(&response).unwrap()
}

async fn native_exchange(port: u16, request: RendezvousMessage) -> RendezvousMessage {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut framed = Framed::new(stream, BytesCodec::new());
    let offer = framed.next().await.unwrap().unwrap();
    let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
    assert!(matches!(
        offer.union,
        Some(rendezvous_message::Union::KeyExchange(_))
    ));
    framed
        .send(Bytes::from(request.write_to_bytes().unwrap()))
        .await
        .unwrap();
    let response = timeout(3_000, framed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    RendezvousMessage::parse_from_bytes(&response).unwrap()
}

async fn secure_exchange(port: u16, request: RendezvousMessage) -> RendezvousMessage {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut framed = Framed::new(stream, BytesCodec::new());
    let offer = framed.next().await.unwrap().unwrap();
    let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
    let Some(rendezvous_message::Union::KeyExchange(exchange)) = offer.union else {
        panic!("server did not offer Secure TCP");
    };
    let signed = &exchange.keys[0];
    assert!(signed.len() >= sign::SIGNATUREBYTES + box_::PUBLICKEYBYTES);
    let mut server_public = [0_u8; box_::PUBLICKEYBYTES];
    server_public.copy_from_slice(&signed[signed.len() - box_::PUBLICKEYBYTES..]);
    let server_public = box_::PublicKey(server_public);
    let (client_public, client_secret) = box_::gen_keypair();
    let key = secretbox::gen_key();
    let sealed = box_::seal(
        &key.0,
        &box_::Nonce([0_u8; box_::NONCEBYTES]),
        &server_public,
        &client_secret,
    );
    let mut response = RendezvousMessage::new();
    response.set_key_exchange(KeyExchange {
        keys: vec![client_public.0.to_vec().into(), sealed.into()],
        ..Default::default()
    });
    framed
        .send(Bytes::from(response.write_to_bytes().unwrap()))
        .await
        .unwrap();

    let mut nonce = secretbox::Nonce([0_u8; secretbox::NONCEBYTES]);
    nonce.0[..8].copy_from_slice(&1_u64.to_le_bytes());
    framed
        .send(Bytes::from(secretbox::seal(
            &request.write_to_bytes().unwrap(),
            &nonce,
            &key,
        )))
        .await
        .unwrap();
    let encrypted = timeout(3_000, framed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let plaintext = secretbox::open(&encrypted, &nonce, &key).unwrap();
    RendezvousMessage::parse_from_bytes(&plaintext).unwrap()
}

async fn websocket_exchange(port: u16, request: RendezvousMessage) -> RendezvousMessage {
    let (mut websocket, _) = connect_async(format!("ws://127.0.0.1:{}/ws/id", port + 2))
        .await
        .unwrap();
    websocket
        .send(Message::Binary(request.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
    let response = timeout(3_000, websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    RendezvousMessage::parse_from_bytes(&response.into_data()).unwrap()
}

fn reserve_hbbs_ports() -> u16 {
    for port in 30_001..60_000u16 {
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

async fn wait_until_listening(address: SocketAddr) {
    for _ in 0..200 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("HBBS did not listen on {address}");
}
