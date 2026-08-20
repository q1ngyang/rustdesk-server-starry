use hbb_common::{
    timeout,
    tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        runtime::Builder,
        time::{sleep, Duration},
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MAGIC: &[u8] = b"STARRYCTL/1\n";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const LOCAL_AUTH_TOKEN: &str = "localControlTokenForOrdinaryIntegration01";

struct TestEnvironment {
    child: Child,
    root: PathBuf,
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn framed_loopback_control_is_structured_bounded_and_legacy_is_disabled() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let port = reserve_hbbs_ports();
            let root = std::env::temp_dir().join(format!(
                "starry-local-control-{}-{port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let config = root.join("config.yaml");
            fs::write(
                &config,
                "version: 3\nrelay_servers:\n  - relay-a.example.com:21117\n",
            )
            .unwrap();
            let token_file = root.join("local-control.token");
            fs::write(&token_file, format!("{LOCAL_AUTH_TOKEN}\n")).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&token_file, fs::Permissions::from_mode(0o600)).unwrap();
            let child = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(port.to_string())
                .arg(format!("--starry-config={}", config.display()))
                .env("TEST_HBBS", "no")
                .env("RUST_LOG", "warn")
                .env("STARRY_LOCAL_CONTROL_TOKEN_FILE", &token_file)
                .current_dir(&root)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = TestEnvironment { child, root };
            let address = SocketAddr::from(([127, 0, 0, 1], port - 1));
            wait_until_listening(address).await;

            let capabilities = request(
                address,
                json!({
                    "request_id": "request-capabilities",
                    "method": "capabilities",
                    "params": {}
                }),
                true,
            )
            .await;
            assert_eq!(capabilities["ok"], true);
            assert_eq!(
                capabilities["result"]["protocol"]["name"],
                "starry-local-control"
            );
            let denied = request_with_token(
                address,
                json!({
                    "request_id": "request-denied",
                    "method": "runtime.reload",
                    "params": {}
                }),
                "wrongLocalControlTokenForRegression01",
                false,
            )
            .await;
            assert_eq!(denied["ok"], false);
            assert_eq!(denied["error"]["code"], "LOCAL_CONTROL_UNAUTHORIZED");

            let relays = request(
                address,
                json!({"request_id": "request-relays", "method": "relays", "params": {}}),
                false,
            )
            .await;
            assert_eq!(relays["ok"], true);
            assert_eq!(relays["result"]["config_generation"], 1);
            assert_eq!(relays["result"]["relays"][0]["configured_order"], 0);
            assert_eq!(relays["result"]["relays"][0]["native"]["state"], "online");

            let simulation = request(
                address,
                json!({
                    "request_id": "request-simulation",
                    "method": "allocation.simulate",
                    "params": {
                        "client_a": {"ip": "192.0.2.10"},
                        "client_b": {"ip": "2001:db8::10"},
                        "transport": "native",
                        "explain": true,
                        "expected_config_generation": 1
                    }
                }),
                false,
            )
            .await;
            assert_eq!(simulation["ok"], true);
            assert_eq!(simulation["result"]["selection"]["non_binding"], true);
            assert_eq!(simulation["result"]["candidates"][0]["eligible"], true);
            let repeated_simulation = request(
                address,
                json!({
                    "request_id": "request-simulation-repeat",
                    "method": "allocation.simulate",
                    "params": {
                        "client_a": {"ip": "192.0.2.10"},
                        "client_b": {"ip": "2001:db8::10"},
                        "transport": "native",
                        "explain": true,
                        "expected_config_generation": 1
                    }
                }),
                false,
            )
            .await;
            assert_eq!(
                repeated_simulation["result"], simulation["result"],
                "simulation must not mutate rotation, config, or health state"
            );

            fs::write(
                &config,
                r#"version: 3
relay_servers:
  - relay-b.example.com:21117
connection_auth:
  mode: audit
  issuer: https://api.example.com
  audience: rustdesk-connect
  jwks:
    file: auth/jwks.json
"#,
            )
            .unwrap();
            let rejected_auth = request(
                address,
                json!({
                    "request_id": "request-auth-reload",
                    "method": "runtime.reload",
                    "params": {}
                }),
                false,
            )
            .await;
            assert_eq!(rejected_auth["ok"], false);
            assert!(rejected_auth["error"]["detail"]
                .as_str()
                .unwrap()
                .contains("JWKS"));
            let retained = request(
                address,
                json!({"request_id": "request-retained", "method": "relays", "params": {}}),
                false,
            )
            .await;
            assert_eq!(retained["result"]["config_generation"], 1);
            assert_eq!(
                retained["result"]["relays"][0]["id"],
                "relay-a.example.com:21117"
            );

            let oversized = oversized_request(address).await;
            assert_eq!(oversized["ok"], false);
            assert_eq!(oversized["error"]["code"], "LOCAL_CONTROL_PROTOCOL_ERROR");

            let mut legacy = TcpStream::connect(address).await.unwrap();
            legacy.write_all(b"h").await.unwrap();
            let mut response = vec![0_u8; 4096];
            let count = timeout(2_000, legacy.read(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                count, 0,
                "legacy text control must be closed without a response"
            );

            let non_loopback = local_ip_address::local_ip().unwrap();
            assert!(!non_loopback.is_loopback());
            let mut remote_path = TcpStream::connect(SocketAddr::new(non_loopback, port - 1))
                .await
                .unwrap();
            let payload = serde_json::to_vec(&json!({
                "request_id": "must-not-be-served",
                "method": "status",
                "params": {}
            }))
            .unwrap();
            let mut framed = MAGIC.to_vec();
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            remote_path.write_all(&framed).await.unwrap();
            let mut alleged_magic = [0_u8; MAGIC.len()];
            if let Ok(Ok(read)) = timeout(300, remote_path.read(&mut alleged_magic)).await {
                assert_ne!(&alleged_magic[..read], &MAGIC[..read]);
            }
        });
}

async fn request(address: SocketAddr, value: Value, fragmented: bool) -> Value {
    request_with_token(address, value, LOCAL_AUTH_TOKEN, fragmented).await
}

async fn request_with_token(
    address: SocketAddr,
    value: Value,
    auth_token: &str,
    fragmented: bool,
) -> Value {
    let mut value = value;
    value["auth_token"] = Value::String(auth_token.to_owned());
    let payload = serde_json::to_vec(&value).unwrap();
    let mut frame = MAGIC.to_vec();
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    let mut stream = TcpStream::connect(address).await.unwrap();
    if fragmented {
        for chunk in frame.chunks(2) {
            stream.write_all(chunk).await.unwrap();
            hbb_common::tokio::task::yield_now().await;
        }
    } else {
        stream.write_all(&frame).await.unwrap();
    }
    read_response(&mut stream).await
}

async fn oversized_request(address: SocketAddr) -> Value {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(MAGIC).await.unwrap();
    stream
        .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
        .await
        .unwrap();
    read_response(&mut stream).await
}

async fn read_response(stream: &mut TcpStream) -> Value {
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
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await.unwrap();
    serde_json::from_slice(&payload).unwrap()
}

fn reserve_hbbs_ports() -> u16 {
    for port in 30_001..40_000_u16 {
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
    for _ in 0..250 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("HBBS did not listen on {address}");
}
