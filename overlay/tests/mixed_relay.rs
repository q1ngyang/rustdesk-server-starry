use hbb_common::{
    bytes::Bytes,
    futures_util::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::{RendezvousMessage, RequestRelay},
    tcp::FramedStream,
    timeout,
    tokio::{
        net::TcpStream,
        runtime::Builder,
        time::{sleep, Duration},
    },
};
use std::{
    fs,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
};
use tungstenite::Message;

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

        let child = Command::new(env!("CARGO_BIN_EXE_hbbr"))
            .arg("--port")
            .arg(port.to_string())
            .current_dir(&state_dir)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let _relay = RelayProcess { child, state_dir };
        wait_until_listening(port).await;
        wait_until_listening(port + 2).await;

        run_pair(port, "starry-mixed-ws-first", true).await;
        run_pair(port, "starry-mixed-native-first", false).await;
    });
}

async fn run_pair(port: u16, uuid: &str, websocket_first: bool) {
    let ws_url = format!("ws://127.0.0.1:{}/ws/relay", port + 2);
    let (mut websocket, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
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
            .send(Message::Binary(request.write_to_bytes().unwrap()))
            .await
            .unwrap();
        sleep(Duration::from_millis(100)).await;
        native.send(&request).await.unwrap();
    } else {
        native.send(&request).await.unwrap();
        sleep(Duration::from_millis(100)).await;
        websocket
            .send(Message::Binary(request.write_to_bytes().unwrap()))
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
        .send(Message::Binary(websocket_payload.to_vec()))
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
