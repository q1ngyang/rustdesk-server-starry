use hbb_common::{protobuf::Message as _, rendezvous_proto::FastRelayAuthorization};
use sha2::Digest as _;
use sodiumoxide::crypto::sign;
use std::{
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ROLE_CONTROLLER: u8 = 1;
const ROLE_TARGET: u8 = 2;
const SESSION_ID: u64 = 0x1020_3040_5060_7080;
const ALLOCATION_ID: [u8; 16] = [0x41; 16];

struct RelayProcess {
    child: Child,
}

impl Drop for RelayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn real_hbbr_recovers_udp_binds_both_roles_forwards_akf1_and_keeps_tcp_alive() {
    sodiumoxide::init().unwrap();
    let relay_port = reserve_relay_port_pair();
    let udp_blocker = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp_blocker.local_addr().unwrap().port();
    let relay_server = format!("127.0.0.1:{relay_port}");
    let (public, secret) = sign::gen_keypair();
    let temporary_root = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/var/tmp"));
    let state = tempfile::Builder::new()
        .prefix("starry-fast-media-process-")
        .tempdir_in(temporary_root)
        .unwrap();
    let _relay = spawn_relay(state.path(), relay_port, udp_port, &relay_server, &public);
    wait_tcp(relay_port);
    assert!(TcpStream::connect_timeout(
        &format!("127.0.0.1:{relay_port}").parse().unwrap(),
        Duration::from_secs(1),
    )
    .is_ok());
    // The optional UDP listener initially cannot bind, but the reliable Relay
    // is already usable. Releasing the port lets the supervised listener
    // recover without restarting HBBR or dropping the desktop session.
    thread::sleep(Duration::from_millis(600));
    drop(udp_blocker);

    let controller = udp_client();
    let target = udp_client();
    let relay_udp: SocketAddr = format!("127.0.0.1:{udp_port}").parse().unwrap();
    let now = unix_seconds();

    // A valid cookie with a grant for the opposite role is still rejected.
    let nonce = [0x11; 16];
    let cookie = hello_cookie(&controller, relay_udp, ROLE_CONTROLLER, nonce);
    let wrong_role = signed_grant(&secret, ROLE_TARGET, &relay_server, udp_port, now + 90);
    controller
        .send_to(
            &bind_packet(ROLE_CONTROLLER, nonce, cookie, &wrong_role),
            relay_udp,
        )
        .unwrap();
    assert!(receive(&controller).is_none());
    assert!(TcpStream::connect_timeout(
        &format!("127.0.0.1:{relay_port}").parse().unwrap(),
        Duration::from_secs(1),
    )
    .is_ok());

    let controller_bootstrap = bind_role(
        &controller,
        relay_udp,
        ROLE_CONTROLLER,
        [0x21; 16],
        &secret,
        &relay_server,
        udp_port,
        now,
    );
    let target_bootstrap = bind_role(
        &target,
        relay_udp,
        ROLE_TARGET,
        [0x22; 16],
        &secret,
        &relay_server,
        udp_port,
        now,
    );

    let akf1 = encrypted_akf1(ROLE_TARGET, 1);
    let mut outer = header(5, ROLE_TARGET);
    outer.extend_from_slice(&akf1);
    target.send_to(&outer, relay_udp).unwrap();
    let forwarded = receive(&controller).expect("controller must receive stripped AKF1");
    assert_eq!(forwarded, akf1);
    assert_eq!(&forwarded[..4], b"AKF1");
    assert_ne!(&forwarded, &outer, "HBBR must strip only the AKR1 envelope");

    // Replaying one encrypted-media sequence is fail closed without affecting
    // the reliable TCP Relay listener.
    target.send_to(&outer, relay_udp).unwrap();
    assert!(receive(&controller).is_none());

    let controller_renewed = signed_renewal_grant(
        &secret,
        ROLE_CONTROLLER,
        &relay_server,
        udp_port,
        now + 150,
        1,
        &controller_bootstrap,
    );
    let target_renewed = signed_renewal_grant(
        &secret,
        ROLE_TARGET,
        &relay_server,
        udp_port,
        now + 150,
        1,
        &target_bootstrap,
    );
    let controller_roamed = udp_client();
    bind_signed_role(
        &controller_roamed,
        relay_udp,
        ROLE_CONTROLLER,
        [0x31; 16],
        &controller_renewed,
    );
    bind_signed_role(&target, relay_udp, ROLE_TARGET, [0x32; 16], &target_renewed);
    target.send_to(&outer, relay_udp).unwrap();
    assert!(
        receive(&controller_roamed).is_none(),
        "renewal/rebind must preserve the AKF1 replay window"
    );
    let akf1_after_renewal = encrypted_akf1(ROLE_TARGET, 2);
    let mut outer_after_renewal = header(5, ROLE_TARGET);
    outer_after_renewal.extend_from_slice(&akf1_after_renewal);
    target.send_to(&outer_after_renewal, relay_udp).unwrap();
    assert_eq!(
        receive(&controller_roamed).expect("renewed controller must receive AKF1"),
        akf1_after_renewal
    );
    assert!(TcpStream::connect_timeout(
        &format!("127.0.0.1:{relay_port}").parse().unwrap(),
        Duration::from_secs(1),
    )
    .is_ok());
}

fn spawn_relay(
    state_dir: &Path,
    relay_port: u16,
    udp_port: u16,
    relay_server: &str,
    public: &sign::PublicKey,
) -> RelayProcess {
    let child = Command::new(env!("CARGO_BIN_EXE_hbbr"))
        .arg("--port")
        .arg(relay_port.to_string())
        .arg("--key")
        .arg(base64::encode(public.as_ref()))
        .env("STARRY_RELAY_FAST_MEDIA_UDP_PORT", udp_port.to_string())
        .env("STARRY_RELAY_PUBLIC_ENDPOINT", relay_server)
        .env("STARRY_RELAY_FAST_MEDIA_MAX_ALLOCATIONS", "32")
        .env("STARRY_RELAY_FAST_MEDIA_PER_IP_PACKETS_PER_SECOND", "1000")
        .env("STARRY_RELAY_FAST_MEDIA_GLOBAL_PACKETS_PER_SECOND", "10000")
        .current_dir(state_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    RelayProcess { child }
}

#[allow(clippy::too_many_arguments)]
fn bind_role(
    socket: &UdpSocket,
    relay: SocketAddr,
    role: u8,
    nonce: [u8; 16],
    secret: &sign::SecretKey,
    relay_server: &str,
    udp_port: u16,
    now: u64,
) -> Vec<u8> {
    let signed = signed_grant(secret, role, relay_server, udp_port, now + 90);
    bind_signed_role(socket, relay, role, nonce, &signed);
    signed
}

fn bind_signed_role(
    socket: &UdpSocket,
    relay: SocketAddr,
    role: u8,
    nonce: [u8; 16],
    signed: &[u8],
) {
    let cookie = hello_cookie(socket, relay, role, nonce);
    socket
        .send_to(&bind_packet(role, nonce, cookie, signed), relay)
        .unwrap();
    let bound = receive(socket).expect("HBBR must return Bound");
    assert_eq!(bound.len(), 32);
    assert_eq!(&bound[..4], b"AKR1");
    assert_eq!(bound[5], 4);
    assert_eq!(bound[6], role);
}

fn hello_cookie(socket: &UdpSocket, relay: SocketAddr, role: u8, nonce: [u8; 16]) -> [u8; 8] {
    let mut hello = header(1, role);
    hello.extend_from_slice(&nonce);
    hello.extend_from_slice(&[0; 8]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        socket.send_to(&hello, relay).unwrap();
        if let Some(response) = receive(socket) {
            assert_eq!(response.len(), 56);
            assert_eq!(response[5], 2);
            assert_eq!(response[6], role);
            assert_eq!(&response[32..48], nonce.as_slice());
            return response[48..56].try_into().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "FastMedia UDP listener did not start"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn signed_grant(
    secret: &sign::SecretKey,
    role: u8,
    relay_server: &str,
    udp_port: u16,
    expires_at: u64,
) -> Vec<u8> {
    let payload = FastRelayAuthorization {
        version: 1,
        session_uuid: "process-fast-media-session".to_owned(),
        expires_at,
        allow_fast_compat: true,
        allow_fast_media_v1: true,
        max_bitrate_kbps: 50_000,
        relay_udp_protocol: 1,
        relay_server: relay_server.to_owned(),
        relay_udp_port: u32::from(udp_port),
        relay_allocation_id: ALLOCATION_ID.to_vec().into(),
        relay_max_datagram: 1_200,
        relay_endpoint_role: u32::from(role),
        fast_media_relay_renewal: 1,
        ..Default::default()
    }
    .write_to_bytes()
    .unwrap();
    sign::sign(&payload, secret)
}

#[allow(clippy::too_many_arguments)]
fn signed_renewal_grant(
    secret: &sign::SecretKey,
    role: u8,
    relay_server: &str,
    udp_port: u16,
    expires_at: u64,
    renewal_sequence: u64,
    previous: &[u8],
) -> Vec<u8> {
    let payload = FastRelayAuthorization {
        version: 1,
        session_uuid: "process-fast-media-session".to_owned(),
        expires_at,
        allow_fast_compat: true,
        allow_fast_media_v1: true,
        max_bitrate_kbps: 50_000,
        relay_udp_protocol: 1,
        relay_server: relay_server.to_owned(),
        relay_udp_port: u32::from(udp_port),
        relay_allocation_id: ALLOCATION_ID.to_vec().into(),
        relay_max_datagram: 1_200,
        relay_endpoint_role: u32::from(role),
        fast_media_relay_renewal: 1,
        relay_session_id: SESSION_ID,
        renewal_sequence,
        previous_authorization_sha256: sha2::Sha256::digest(previous).to_vec().into(),
        ..Default::default()
    }
    .write_to_bytes()
    .unwrap();
    sign::sign(&payload, secret)
}

fn bind_packet(role: u8, nonce: [u8; 16], cookie: [u8; 8], signed: &[u8]) -> Vec<u8> {
    let mut packet = header(3, role);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&cookie);
    packet.extend_from_slice(&(signed.len() as u16).to_le_bytes());
    packet.extend_from_slice(signed);
    packet
}

fn header(kind: u8, role: u8) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32);
    packet.extend_from_slice(b"AKR1");
    packet.push(1);
    packet.push(kind);
    packet.push(role);
    packet.push(0);
    packet.extend_from_slice(&SESSION_ID.to_le_bytes());
    packet.extend_from_slice(&ALLOCATION_ID);
    packet
}

fn encrypted_akf1(role: u8, sequence: u64) -> Vec<u8> {
    // The bytes after the public AKF1 routing prefix are intentionally opaque;
    // HBBR validates only public invariants and never receives a media key.
    let mut packet = vec![0x5a; 22 + 16 + 51];
    packet[..4].copy_from_slice(b"AKF1");
    packet[4] = 1;
    packet[5] = role - 1;
    packet[6..14].copy_from_slice(&SESSION_ID.to_le_bytes());
    packet[14..22].copy_from_slice(&sequence.to_le_bytes());
    packet
}

fn udp_client() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    socket
}

fn receive(socket: &UdpSocket) -> Option<Vec<u8>> {
    let mut buffer = vec![0_u8; 8192];
    match socket.recv_from(&mut buffer) {
        Ok((size, _)) => {
            buffer.truncate(size);
            Some(buffer)
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            None
        }
        Err(error) => panic!("UDP receive failed: {error}"),
    }
}

fn reserve_relay_port_pair() -> u16 {
    for _ in 0..100 {
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = first.local_addr().unwrap().port();
        if port <= u16::MAX - 2 {
            if let Ok(second) = TcpListener::bind(("127.0.0.1", port + 2)) {
                drop(second);
                drop(first);
                return port;
            }
        }
    }
    panic!("unable to reserve HBBR TCP/WS port pair")
}

fn wait_tcp(port: u16) {
    let endpoint = format!("127.0.0.1:{port}").parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&endpoint, Duration::from_millis(100)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("HBBR TCP listener did not start")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
