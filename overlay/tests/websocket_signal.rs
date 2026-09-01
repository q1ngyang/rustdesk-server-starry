use base64::{encode_config, URL_SAFE_NO_PAD};
use hbb_common::{
    bytes::Bytes,
    futures_util::{SinkExt, StreamExt},
    protobuf::{Message as _, MessageField},
    rendezvous_proto::{
        punch_hole_response, register_pk_response, rendezvous_message, DeactivatePeer,
        DeactivatePeerResponse, FastRelayAuthorization, NatType, PunchHole, PunchHoleRequest,
        PunchHoleSent, RegisterPeer, RegisterPk, RegisterPkResponse, RelayProbeReport,
        RelayProbeResult, RelayQualityCancel, RelayQualityOffer, RelayResponse, RendezvousMessage,
        RequestRelay,
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
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::{auth, sign};
use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
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

static NEXT_HBBS_PORT: AtomicUsize = AtomicUsize::new(30_001);
static NEXT_HBBR_PORT: AtomicUsize = AtomicUsize::new(40_000);

struct TestEnvironment {
    children: Vec<Child>,
    tasks: Vec<JoinHandle<()>>,
    root: PathBuf,
}

#[derive(Clone)]
struct ActivationReady {
    epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
    route_generation: u64,
}

#[test]
fn hbbs_signs_one_fast_compat_grant_after_final_quality_selection() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let root = std::env::temp_dir().join(format!(
                "starry-fast-relay-{}-{hbbs_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let hbbs_dir = root.join("hbbs");
            fs::create_dir_all(hbbs_dir.join("auth")).unwrap();

            let (probe_port, probe_port_b, ca_path, telemetry_secret, probe_count, probe_task) =
                start_wss_probe(&root).await;
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
            let token = connection_token(&auth_secret);
            let relay_a = "localhost:21117";
            let relay_b = "localhost:21118";
            let config_path = hbbs_dir.join("config.yaml");
            fs::write(
                &config_path,
                fast_relay_config(
                    relay_a,
                    relay_b,
                    probe_port,
                    probe_port_b,
                    &telemetry_secret,
                ),
            )
            .unwrap();
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
                children: vec![hbbs],
                tasks: vec![probe_task],
                root,
            };

            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port + 2))).await;
            // Startup performs a staged activation and probes both endpoints
            // for each generation. Wait for the final generation's full cycle.
            wait_for_wss_probe_count(&probe_count, 4).await;
            let server_key = wait_for_server_key(&hbbs_dir).await;
            let public_bytes = base64::decode(&server_key).unwrap();
            let server_public = sign::PublicKey::from_slice(&public_bytes).unwrap();
            let controller_id = "fast-controller-001";
            let target_id = "fast-target-000001";
            let session_uuid = "fast-relay-session-0001";
            let mut controller = connect_registered_websocket(hbbs_port, controller_id, 0x61).await;
            let mut target = connect_registered_websocket(hbbs_port, target_id, 0x62).await;

            let mut preflight = punch_request(target_id, &server_key, &token);
            let Some(rendezvous_message::Union::PunchHoleRequest(request)) =
                preflight.union.as_mut()
            else {
                panic!("quality preflight did not contain PunchHoleRequest");
            };
            request.relay_quality_protocol = 1;
            controller
                .send(Message::Binary(preflight.write_to_bytes().unwrap().into()))
                .await
                .unwrap();
            let punch = match receive_protocol(&mut target, 5_000).await.unwrap().union {
                Some(rendezvous_message::Union::PunchHole(punch)) => punch,
                other => panic!("expected quality PunchHole, got {other:?}"),
            };
            let target_offer = punch.relay_quality_offer.as_ref().unwrap().clone();
            assert_eq!(target_offer.strategy, 1);
            assert_eq!(target_offer.stage, 1);
            assert_eq!(target_offer.candidates.len(), 1);
            let target_report = successful_report(&target_offer, 35, 2);
            let mut sent = RendezvousMessage::new();
            sent.set_punch_hole_sent(PunchHoleSent {
                socket_addr: punch.socket_addr,
                id: target_id.to_owned(),
                relay_server: punch.relay_server,
                nat_type: NatType::SYMMETRIC.into(),
                relay_quality_report: MessageField::some(target_report),
                ..Default::default()
            });
            target
                .send(Message::Binary(sent.write_to_bytes().unwrap().into()))
                .await
                .unwrap();

            let response = match receive_protocol(&mut controller, 5_000)
                .await
                .unwrap()
                .union
            {
                Some(rendezvous_message::Union::PunchHoleResponse(response)) => response,
                other => panic!("expected quality PunchHoleResponse, got {other:?}"),
            };
            let controller_offer = response.relay_quality_offer.as_ref().unwrap().clone();
            assert_eq!(controller_offer.allocation_id, target_offer.allocation_id);
            assert!(!response.relay_quality_peer_report.results.is_empty());
            let controller_report = failed_report(&controller_offer, 1);

            let mut first = RendezvousMessage::new();
            first.set_request_relay(RequestRelay {
                id: target_id.to_owned(),
                uuid: session_uuid.to_owned(),
                relay_server: relay_b.to_owned(),
                token: token.clone(),
                relay_quality_report: MessageField::some(controller_report),
                relay_quality_allocation_id: controller_offer.allocation_id.clone(),
                fast_relay_authorization: Bytes::from_static(b"client-supplied-grant"),
                ..Default::default()
            });
            controller
                .send(Message::Binary(first.write_to_bytes().unwrap().into()))
                .await
                .unwrap();

            let controller_expansion = match receive_protocol(&mut controller, 5_000)
                .await
                .unwrap()
                .union
            {
                Some(rendezvous_message::Union::RelayQualityStageOffer(offer)) => offer,
                other => panic!("expected controller expansion offer, got {other:?}"),
            };
            let target_expansion = match receive_protocol(&mut target, 5_000).await.unwrap().union {
                Some(rendezvous_message::Union::RelayQualityStageOffer(offer)) => offer,
                other => panic!("expected target expansion offer, got {other:?}"),
            };
            assert_eq!(controller_expansion.stage, 2);
            assert_eq!(controller_expansion.candidates.len(), 1);
            assert_eq!(controller_expansion.candidates[0].relay_server, relay_b);
            assert_eq!(
                controller_expansion.stage_token,
                target_expansion.stage_token
            );

            let target_expanded_report = successful_report(&target_expansion, 30, 2);
            let mut target_stage = RendezvousMessage::new();
            target_stage.set_relay_quality_stage_report(target_expanded_report);
            target
                .send(Message::Binary(
                    target_stage.write_to_bytes().unwrap().into(),
                ))
                .await
                .unwrap();
            let controller_expanded_report = successful_report(&controller_expansion, 32, 1);
            let mut controller_stage = RendezvousMessage::new();
            controller_stage.set_relay_quality_stage_report(controller_expanded_report);
            controller
                .send(Message::Binary(
                    controller_stage.write_to_bytes().unwrap().into(),
                ))
                .await
                .unwrap();

            let controller_decision = match receive_protocol(&mut controller, 5_000)
                .await
                .unwrap()
                .union
            {
                Some(rendezvous_message::Union::RelayQualityStageDecision(decision)) => decision,
                other => panic!("expected controller final decision, got {other:?}"),
            };
            let target_decision = match receive_protocol(&mut target, 5_000).await.unwrap().union {
                Some(rendezvous_message::Union::RelayQualityStageDecision(decision)) => decision,
                other => panic!("expected target final decision, got {other:?}"),
            };
            assert_eq!(controller_decision, target_decision);
            assert_eq!(controller_decision.relay_server, relay_b);
            assert_eq!(controller_decision.reason_code, 2);

            let mut committed = RendezvousMessage::new();
            committed.set_request_relay(RequestRelay {
                id: target_id.to_owned(),
                uuid: session_uuid.to_owned(),
                relay_server: relay_a.to_owned(),
                token: token.clone(),
                relay_quality_allocation_id: controller_offer.allocation_id.clone(),
                fast_relay_authorization: Bytes::from_static(b"client-supplied-grant"),
                ..Default::default()
            });
            controller
                .send(Message::Binary(committed.write_to_bytes().unwrap().into()))
                .await
                .unwrap();
            let first_request = expect_fast_request(
                receive_protocol(&mut target, 5_000).await.unwrap(),
                session_uuid,
                &server_public,
            );
            let signed = first_request.fast_relay_authorization.clone();
            assert_ne!(signed.as_ref(), b"client-supplied-grant");

            send_fast_response(&mut target, &first_request, b"target-supplied-grant").await;
            let first_response = expect_fast_response(
                receive_protocol(&mut controller, 5_000).await.unwrap(),
                &signed,
            );
            assert_eq!(first_response.relay_server, first_request.relay_server);

            let mut retry = RendezvousMessage::new();
            retry.set_request_relay(RequestRelay {
                id: target_id.to_owned(),
                uuid: session_uuid.to_owned(),
                relay_server: relay_b.to_owned(),
                token: token.clone(),
                relay_quality_allocation_id: controller_offer.allocation_id,
                ..Default::default()
            });
            controller
                .send(Message::Binary(retry.write_to_bytes().unwrap().into()))
                .await
                .unwrap();
            let retry_request = expect_fast_request(
                receive_protocol(&mut target, 5_000).await.unwrap(),
                session_uuid,
                &server_public,
            );
            assert_eq!(retry_request.fast_relay_authorization, signed);
            assert_eq!(
                retry_request.relay_quality_decision,
                first_request.relay_quality_decision
            );
            send_fast_response(&mut target, &retry_request, b"different-untrusted-grant").await;
            let retry_response = expect_fast_response(
                receive_protocol(&mut controller, 5_000).await.unwrap(),
                &signed,
            );
            assert_eq!(retry_response.fast_relay_authorization, signed);

            let mut p2p_preflight = punch_request(target_id, &server_key, &token);
            let Some(rendezvous_message::Union::PunchHoleRequest(request)) =
                p2p_preflight.union.as_mut()
            else {
                panic!("P2P cancellation preflight did not contain PunchHoleRequest");
            };
            request.relay_quality_protocol = 1;
            controller
                .send(Message::Binary(
                    p2p_preflight.write_to_bytes().unwrap().into(),
                ))
                .await
                .unwrap();
            let p2p_offer = match receive_protocol(&mut target, 5_000).await.unwrap().union {
                Some(rendezvous_message::Union::PunchHole(punch)) => {
                    punch.relay_quality_offer.into_option().unwrap()
                }
                other => panic!("expected P2P cancellation PunchHole, got {other:?}"),
            };
            let mut cancel = RendezvousMessage::new();
            cancel.set_relay_quality_cancel(RelayQualityCancel {
                protocol_version: 1,
                allocation_id: p2p_offer.allocation_id,
                stage: p2p_offer.stage,
                stage_token: p2p_offer.stage_token,
                reason_code: 1,
                endpoint_role: 2,
                ..Default::default()
            });
            let cancel_started = Instant::now();
            target
                .send(Message::Binary(cancel.write_to_bytes().unwrap().into()))
                .await
                .unwrap();
            for endpoint in [&mut controller, &mut target] {
                match receive_protocol(endpoint, 1_000).await.unwrap().union {
                    Some(rendezvous_message::Union::RelayQualityCancel(cancel)) => {
                        assert_eq!(cancel.reason_code, 1)
                    }
                    other => panic!("expected propagated P2P cancel, got {other:?}"),
                }
            }
            assert!(cancel_started.elapsed() < Duration::from_secs(1));
        });
}

#[test]
fn profile_activation_is_generation_safe_across_udp_tcp_and_legacy_clients() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let root = std::env::temp_dir().join(format!(
                "starry-profile-native-{}-{hbbs_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let hbbs_dir = root.join("hbbs");
            fs::create_dir_all(&hbbs_dir).unwrap();
            let hbbs = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(hbbs_port.to_string())
                .env("RUST_LOG", "warn")
                .current_dir(&hbbs_dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = TestEnvironment {
                children: vec![hbbs],
                tasks: Vec::new(),
                root,
            };
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port))).await;

            let peer_id = "profile-native-0001";
            let uuid = [0x41; 16];
            let public_key = [0x51; 32];
            let (mut socket_a1, ready_a1) =
                register_native_activation(hbbs_port, peer_id, &uuid, &public_key, 1, [0x61; 16])
                    .await;
            let (mut socket_b, ready_b) =
                register_native_activation(hbbs_port, peer_id, &uuid, &public_key, 2, [0x62; 16])
                    .await;
            assert!(ready_b.route_generation > ready_a1.route_generation);

            // A delayed heartbeat and lease from activation A1 cannot mutate B.
            assert!(renew_native_activation(hbbs_port, peer_id, &mut socket_a1, &ready_a1,).await);
            assert!(!deactivate_udp(hbbs_port, peer_id, &uuid, &mut socket_a1, &ready_a1,).await);
            assert!(!renew_native_activation(hbbs_port, peer_id, &mut socket_b, &ready_b,).await);

            let (mut socket_a2, ready_a2) =
                register_native_activation(hbbs_port, peer_id, &uuid, &public_key, 3, [0x63; 16])
                    .await;
            assert!(ready_a2.route_generation > ready_b.route_generation);
            assert!(!deactivate_tcp(hbbs_port, peer_id, &uuid, &ready_b,).await);
            assert!(!renew_native_activation(hbbs_port, peer_id, &mut socket_a2, &ready_a2,).await);
            assert!(deactivate_udp(hbbs_port, peer_id, &uuid, &mut socket_a2, &ready_a2,).await);

            // A separate official/legacy registration keeps all extension
            // fields at their proto3 defaults and follows the old flow.
            let legacy_id = "profile-legacy-0001";
            let (_legacy, legacy_ready) = register_native_legacy(hbbs_port, legacy_id, 0x71).await;
            assert_eq!(legacy_ready.route_generation, 0);
            assert_eq!(legacy_ready.activation_epoch, 0);
            assert!(legacy_ready.activation_id.is_empty());
            assert!(legacy_ready.route_lease.is_empty());
        });
}

#[test]
fn profile_activation_wss_old_reader_and_a_to_b_to_a_are_generation_safe() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let root = std::env::temp_dir().join(format!(
                "starry-profile-wss-{}-{hbbs_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let hbbs_dir = root.join("hbbs");
            fs::create_dir_all(&hbbs_dir).unwrap();
            let (probe_port, _, ca_path, _, probe_count, probe_task) = start_wss_probe(&root).await;
            let relay = "127.0.0.1:21117";
            let config_path = hbbs_dir.join("config.yaml");
            fs::write(
                &config_path,
                profile_activation_websocket_config(relay, probe_port),
            )
            .unwrap();
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
                children: vec![hbbs],
                tasks: vec![probe_task],
                root,
            };
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port + 2))).await;
            wait_for_wss_probe(&probe_count).await;

            let peer_id = "profile-wss-000001";
            let uuid = [0x81; 16];
            let public_key = [0x91; 32];
            let (mut socket_a1, ready_a1) =
                connect_profile_websocket(hbbs_port, peer_id, &uuid, &public_key, 1, [0xA1; 16])
                    .await;
            let (socket_b, ready_b) =
                connect_profile_websocket(hbbs_port, peer_id, &uuid, &public_key, 2, [0xB1; 16])
                    .await;
            assert!(ready_b.route_generation > ready_a1.route_generation);
            sleep(Duration::from_millis(100)).await;
            assert!(!deactivate_tcp(hbbs_port, peer_id, &uuid, &ready_a1,).await);
            let socket_b = verify_websocket_ping(socket_b, vec![0xB2]).await;

            // Closing the superseded A1 reader cannot remove B.
            let _ = socket_a1.close(None).await;
            sleep(Duration::from_millis(100)).await;
            let socket_b = verify_websocket_ping(socket_b, vec![0xB3]).await;

            let (mut socket_a2, ready_a2) =
                connect_profile_websocket(hbbs_port, peer_id, &uuid, &public_key, 3, [0xA2; 16])
                    .await;
            assert!(ready_a2.route_generation > ready_b.route_generation);
            drop(socket_b);
            sleep(Duration::from_millis(100)).await;
            socket_a2 = verify_websocket_ping(socket_a2, vec![0xA3]).await;

            // A transport retry for the same activation deliberately reuses
            // its lease/generation. The older reader is still distinguished
            // by connection ID and cannot remove the replacement.
            let (mut socket_a2_retry, ready_a2_retry) =
                connect_profile_websocket(hbbs_port, peer_id, &uuid, &public_key, 3, [0xA2; 16])
                    .await;
            assert_eq!(ready_a2_retry.route_lease, ready_a2.route_lease);
            assert_eq!(ready_a2_retry.route_generation, ready_a2.route_generation);
            let _ = socket_a2.close(None).await;
            sleep(Duration::from_millis(100)).await;
            socket_a2_retry = verify_websocket_ping(socket_a2_retry, vec![0xA4]).await;

            // A delayed Native heartbeat carrying the same activation lease
            // cannot migrate the route away from the replacement WSS reader.
            let mut delayed_native = FramedSocket::new("0.0.0.0:0").await.unwrap();
            assert!(
                renew_native_activation(hbbs_port, peer_id, &mut delayed_native, &ready_a2_retry,)
                    .await
            );
            socket_a2_retry = verify_websocket_ping(socket_a2_retry, vec![0xA5]).await;

            let response =
                deactivate_websocket(peer_id, &uuid, &mut socket_a2_retry, &ready_a2_retry).await;
            assert!(response.deactivated);
            assert_eq!(response.activation_id, ready_a2_retry.activation_id);
            assert_eq!(response.route_generation, ready_a2_retry.route_generation);
        });
}

#[test]
fn profile_activation_route_leases_are_node_local() {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let port_a = reserve_hbbs_ports();
            let port_b = reserve_hbbs_ports_excluding(port_a);
            let root = std::env::temp_dir().join(format!(
                "starry-profile-multinode-{}-{port_a}-{port_b}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let node_a = root.join("node-a");
            let node_b = root.join("node-b");
            fs::create_dir_all(&node_a).unwrap();
            fs::create_dir_all(&node_b).unwrap();
            let child_a = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(port_a.to_string())
                .env("RUST_LOG", "warn")
                .current_dir(&node_a)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let child_b = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(port_b.to_string())
                .env("RUST_LOG", "warn")
                .current_dir(&node_b)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = TestEnvironment {
                children: vec![child_a, child_b],
                tasks: Vec::new(),
                root,
            };
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], port_a))).await;
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], port_b))).await;

            let peer_id = "profile-multinode-1";
            let uuid = [0xC1; 16];
            let public_key = [0xD1; 32];
            let (mut socket_a, ready_a) =
                register_native_activation(port_a, peer_id, &uuid, &public_key, 1, [0xE1; 16])
                    .await;
            let (mut socket_b, ready_b) =
                register_native_activation(port_b, peer_id, &uuid, &public_key, 1, [0xE1; 16])
                    .await;
            assert_ne!(ready_a.route_lease, ready_b.route_lease);
            assert!(deactivate_udp(port_a, peer_id, &uuid, &mut socket_a, &ready_a,).await);
            assert!(!deactivate_udp(port_b, peer_id, &uuid, &mut socket_b, &ready_a,).await);
            assert!(!renew_native_activation(port_b, peer_id, &mut socket_b, &ready_b,).await);
        });
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

            let (probe_port, _, ca_path, _, probe_count, probe_task) = start_wss_probe(&root).await;
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
    ensure_websocket_load_nofile_limit();
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

            let (probe_port, _, ca_path, _, probe_count, probe_task) = start_wss_probe(&root).await;
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

#[cfg(unix)]
fn ensure_websocket_load_nofile_limit() {
    const REQUIRED: libc::rlim_t = 8_192;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let read_result = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    assert_eq!(
        read_result,
        0,
        "cannot read RLIMIT_NOFILE for the 1,000-WebSocket release gate: {}",
        std::io::Error::last_os_error()
    );
    assert!(
        limit.rlim_max >= REQUIRED,
        "the 1,000-WebSocket release gate requires RLIMIT_NOFILE hard limit >= {REQUIRED}, got {}",
        limit.rlim_max
    );
    if limit.rlim_cur < REQUIRED {
        limit.rlim_cur = REQUIRED;
        let write_result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
        assert_eq!(
            write_result,
            0,
            "cannot raise RLIMIT_NOFILE to {REQUIRED} for the 1,000-WebSocket release gate: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(unix))]
fn ensure_websocket_load_nofile_limit() {}

async fn start_wss_probe(
    root: &Path,
) -> (u16, u16, PathBuf, PathBuf, Arc<AtomicUsize>, JoinHandle<()>) {
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
    let telemetry_secret_path = root.join("test-relay-telemetry.secret");
    let telemetry_secret = b"starry-test-telemetry-secret-32-bytes-minimum";
    fs::write(&telemetry_secret_path, telemetry_secret).unwrap();
    let telemetry_key =
        auth::Key::from_slice(&Sha256::digest(telemetry_secret)).expect("test HMAC key");
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
    let listener_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port_b = listener_b.local_addr().unwrap().port();
    let probe_count = Arc::new(AtomicUsize::new(0));
    let task_probe_count = probe_count.clone();
    let telemetry_sequence = Arc::new(AtomicUsize::new(0));

    let task = hbb_common::tokio::spawn(async move {
        loop {
            let accepted = hbb_common::tokio::select! {
                accepted = listener.accept() => accepted,
                accepted = listener_b.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else {
                break;
            };
            let acceptor = acceptor.clone();
            let probe_count = task_probe_count.clone();
            let telemetry_key = telemetry_key.clone();
            let telemetry_sequence = telemetry_sequence.clone();
            hbb_common::tokio::spawn(async move {
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        eprintln!("test /ws/relay TLS accept failed: {err}");
                        return;
                    }
                };
                let callback =
                    move |request: &http::Request<()>, mut response: http::Response<()>| {
                        let public =
                            request.uri().path() == "/ws/relay" && request.uri().query().is_none();
                        let telemetry = request.uri().path() == "/ws/telemetry"
                            && request.uri().query().is_none();
                        if !public && !telemetry {
                            return Err(http::Response::builder()
                                .status(http::StatusCode::NOT_FOUND)
                                .body(Some("Not Found".to_owned()))
                                .unwrap());
                        }
                        for (name, value) in [
                            ("x-starry-version", "1.1.16-patch-v1.3.0"),
                            ("x-starry-relay-probe-protocol", "1"),
                            ("x-starry-relay-load-protocol", "1"),
                        ] {
                            response.headers_mut().insert(
                                http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                                http::HeaderValue::from_static(value),
                            );
                        }
                        if public {
                            return Ok(response);
                        }
                        let header = |name: &str| {
                            request
                                .headers()
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                        };
                        let Some(timestamp) = header("x-starry-telemetry-timestamp") else {
                            return Err(unauthorized_test_response());
                        };
                        let Some(nonce) = header("x-starry-telemetry-nonce") else {
                            return Err(unauthorized_test_response());
                        };
                        let Some(signature) = header("x-starry-telemetry-auth") else {
                            return Err(unauthorized_test_response());
                        };
                        let canonical = format!(
                            "starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry"
                        );
                        if !test_hmac_matches(signature, canonical.as_bytes(), &telemetry_key) {
                            return Err(unauthorized_test_response());
                        }
                        let sequence = telemetry_sequence.fetch_add(1, Ordering::SeqCst) + 1;
                        let observed_at_unix_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        let payload = encode_config(
                            serde_json::to_vec(&serde_json::json!({
                                "telemetry_schema": 1,
                                "process_instance_id": "integration-relay-instance",
                                "sequence": sequence,
                                "observed_at_unix_ms": observed_at_unix_ms,
                                "uptime_seconds": sequence,
                                "version": "1.1.16-patch-v1.3.0",
                                "relay_probe_protocol": 1,
                                "relay_load_protocol": 1,
                                "load_basis_points": 1000,
                                "active_sessions": 10,
                                "pending_pairs": 2,
                                "capacity_sessions": 100,
                                "bandwidth_bps": 1000000,
                                "bandwidth_ema_alpha_basis_points": 2500,
                                "capacity_bandwidth_bps": 100000000,
                                "draining": false,
                                "admission_open": true,
                                "admission_rejections": 3,
                                "probe_malformed": 4,
                                "probe_unsupported": 5,
                                "probe_rate_limited": 6,
                                "probe_successful": 7,
                                "telemetry_auth_failures": 8
                            }))
                            .unwrap(),
                            URL_SAFE_NO_PAD,
                        );
                        let response_canonical =
                            format!("starry-telemetry-response-v1\n{nonce}\n{payload}");
                        let response_signature = test_hex(
                            auth::authenticate(response_canonical.as_bytes(), &telemetry_key)
                                .as_ref(),
                        );
                        response.headers_mut().insert(
                            "x-starry-telemetry",
                            http::HeaderValue::from_str(&payload).unwrap(),
                        );
                        response.headers_mut().insert(
                            "x-starry-telemetry-auth",
                            http::HeaderValue::from_str(&response_signature).unwrap(),
                        );
                        Ok(response)
                    };
                let mut websocket =
                    match tokio_tungstenite::accept_hdr_async(stream, callback).await {
                        Ok(websocket) => websocket,
                        Err(err) => {
                            eprintln!("test /ws/relay Upgrade failed: {err}");
                            return;
                        }
                    };
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
                probe_count.fetch_add(1, Ordering::SeqCst);
            });
        }
    });
    // The test uses a current-thread runtime. Schedule the accept loop once
    // before the external HBBS process can begin its immediate health probe.
    hbb_common::tokio::task::yield_now().await;
    (
        port,
        port_b,
        ca_path,
        telemetry_secret_path,
        probe_count,
        task,
    )
}

fn unauthorized_test_response() -> http::Response<Option<String>> {
    http::Response::builder()
        .status(http::StatusCode::UNAUTHORIZED)
        .body(Some("Unauthorized".to_owned()))
        .unwrap()
}

fn test_hmac_matches(signature: &str, message: &[u8], key: &auth::Key) -> bool {
    if signature.len() != auth::TAGBYTES * 2 {
        return false;
    }
    let mut decoded = vec![0_u8; auth::TAGBYTES];
    for (index, output) in decoded.iter_mut().enumerate() {
        let Some(high) = test_hex_nibble(signature.as_bytes()[index * 2]) else {
            return false;
        };
        let Some(low) = test_hex_nibble(signature.as_bytes()[index * 2 + 1]) else {
            return false;
        };
        *output = (high << 4) | low;
    }
    auth::Tag::from_slice(&decoded)
        .map(|tag| auth::verify(&tag, message, key))
        .unwrap_or(false)
}

fn test_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn test_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
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

fn profile_activation_websocket_config(relay: &str, probe_port: u16) -> String {
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
"#
    )
}

fn fast_relay_config(
    relay_a: &str,
    relay_b: &str,
    probe_port_a: u16,
    probe_port_b: u16,
    telemetry_secret: &Path,
) -> String {
    format!(
        r#"version: 4
relay_servers:
  - {relay_a}
  - {relay_b}
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
      - relay: {relay_a}
        url: wss://localhost:{probe_port_a}/ws/telemetry
        telemetry_secret_file: {telemetry_secret}
      - relay: {relay_b}
        url: wss://localhost:{probe_port_b}/ws/telemetry
        telemetry_secret_file: {telemetry_secret}
connection_auth:
  mode: enforce
  issuer: https://api.example.test
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  jwks:
    file: auth/jwks.json
relay_quality:
  enabled: true
  strategy: adaptive
  max_candidates: 2
  primary_probe_samples: 3
  primary_accept_score: 1
  primary_max_loss_basis_points: 500
  p2p_probe_grace_ms: 0
  probe_samples: 5
  probe_interval_ms: 20
  probe_timeout_ms: 100
  report_timeout_ms: 4000
  max_telemetry_age_seconds: 30
  allocation_ttl_seconds: 30
  cache_ttl_seconds: 300
  max_allocations: 100
  hysteresis_basis_points: 0
  missing_report_penalty_basis_points: 1000
  rtt_bad_ms: 300
  jitter_bad_ms: 100
  weights:
    rtt: 4000
    jitter: 2000
    loss: 2500
    load: 1500
fast_mode:
  relay:
    fast_compat_enabled: true
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
"#,
        telemetry_secret = telemetry_secret.display()
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

async fn register_native_activation(
    port: u16,
    id: &str,
    uuid: &[u8; 16],
    public_key: &[u8; 32],
    epoch: u64,
    activation_id: [u8; 16],
) -> (FramedSocket, ActivationReady) {
    let mut socket = FramedSocket::new("0.0.0.0:0").await.unwrap();
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let mut register_peer = RendezvousMessage::new();
    register_peer.set_register_peer(RegisterPeer {
        id: id.to_owned(),
        ..Default::default()
    });
    socket.send(&register_peer, server).await.unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    assert!(matches!(
        RendezvousMessage::parse_from_bytes(&bytes).unwrap().union,
        Some(rendezvous_message::Union::RegisterPeerResponse(response)) if response.request_pk
    ));

    let mut register_pk = RendezvousMessage::new();
    register_pk.set_register_pk(RegisterPk {
        id: id.to_owned(),
        uuid: Bytes::copy_from_slice(uuid),
        pk: Bytes::copy_from_slice(public_key),
        activation_epoch: epoch,
        activation_id: Bytes::copy_from_slice(&activation_id),
        ..Default::default()
    });
    socket.send(&register_pk, server).await.unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    let response = match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::RegisterPkResponse(response)) => response,
        other => panic!("expected Profile Ready ACK, got {other:?}"),
    };
    assert_eq!(
        response.result.enum_value().ok(),
        Some(register_pk_response::Result::OK)
    );
    assert_eq!(response.activation_epoch, epoch);
    assert_eq!(response.activation_id.as_ref(), activation_id);
    assert_eq!(response.route_lease.len(), 32);
    assert!(response.route_generation > 0);
    (
        socket,
        ActivationReady {
            epoch,
            activation_id: response.activation_id,
            route_lease: response.route_lease,
            route_generation: response.route_generation,
        },
    )
}

async fn register_native_legacy(
    port: u16,
    id: &str,
    identity_byte: u8,
) -> (FramedSocket, RegisterPkResponse) {
    let mut socket = FramedSocket::new("0.0.0.0:0").await.unwrap();
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let mut register_peer = RendezvousMessage::new();
    register_peer.set_register_peer(RegisterPeer {
        id: id.to_owned(),
        ..Default::default()
    });
    socket.send(&register_peer, server).await.unwrap();
    let _ = socket.next_timeout(3_000).await.unwrap().unwrap();
    let mut register_pk = RendezvousMessage::new();
    register_pk.set_register_pk(RegisterPk {
        id: id.to_owned(),
        uuid: Bytes::from(vec![identity_byte; 16]),
        pk: Bytes::from(vec![identity_byte; 32]),
        ..Default::default()
    });
    socket.send(&register_pk, server).await.unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    let response = match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::RegisterPkResponse(response)) => response,
        other => panic!("expected legacy RegisterPkResponse, got {other:?}"),
    };
    assert_eq!(
        response.result.enum_value().ok(),
        Some(register_pk_response::Result::OK)
    );
    (socket, response)
}

async fn renew_native_activation(
    port: u16,
    id: &str,
    socket: &mut FramedSocket,
    ready: &ActivationReady,
) -> bool {
    let mut renewal = RendezvousMessage::new();
    renewal.set_register_peer(RegisterPeer {
        id: id.to_owned(),
        route_generation: ready.route_generation,
        activation_epoch: ready.epoch,
        activation_id: ready.activation_id.clone(),
        route_lease: ready.route_lease.clone(),
        ..Default::default()
    });
    socket
        .send(&renewal, SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::RegisterPeerResponse(response)) => response.request_pk,
        other => panic!("expected RegisterPeerResponse, got {other:?}"),
    }
}

fn deactivate_message(id: &str, uuid: &[u8; 16], ready: &ActivationReady) -> RendezvousMessage {
    let mut message = RendezvousMessage::new();
    message.set_deactivate_peer(DeactivatePeer {
        id: id.to_owned(),
        network_identity_uuid: Bytes::copy_from_slice(uuid),
        activation_epoch: ready.epoch,
        activation_id: ready.activation_id.clone(),
        route_lease: ready.route_lease.clone(),
        route_generation: ready.route_generation,
        ..Default::default()
    });
    message
}

fn deactivation_result(message: RendezvousMessage) -> DeactivatePeerResponse {
    match message.union {
        Some(rendezvous_message::Union::DeactivatePeerResponse(response)) => response,
        other => panic!("expected DeactivatePeerResponse, got {other:?}"),
    }
}

async fn deactivate_udp(
    port: u16,
    id: &str,
    uuid: &[u8; 16],
    socket: &mut FramedSocket,
    ready: &ActivationReady,
) -> bool {
    socket
        .send(
            &deactivate_message(id, uuid, ready),
            SocketAddr::from(([127, 0, 0, 1], port)),
        )
        .await
        .unwrap();
    let (bytes, _) = socket.next_timeout(3_000).await.unwrap().unwrap();
    let response = deactivation_result(RendezvousMessage::parse_from_bytes(&bytes).unwrap());
    assert_eq!(response.activation_id, ready.activation_id);
    assert_eq!(response.route_generation, ready.route_generation);
    response.deactivated
}

async fn deactivate_tcp(port: u16, id: &str, uuid: &[u8; 16], ready: &ActivationReady) -> bool {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let peer_addr = stream.peer_addr().unwrap();
    let mut framed = FramedStream::from(stream, peer_addr);
    framed
        .send(&deactivate_message(id, uuid, ready))
        .await
        .unwrap();
    let bytes = framed.next_timeout(3_000).await.unwrap().unwrap();
    let response = deactivation_result(RendezvousMessage::parse_from_bytes(&bytes).unwrap());
    assert_eq!(response.activation_id, ready.activation_id);
    assert_eq!(response.route_generation, ready.route_generation);
    response.deactivated
}

async fn connect_profile_websocket(
    port: u16,
    id: &str,
    uuid: &[u8; 16],
    public_key: &[u8; 32],
    epoch: u64,
    activation_id: [u8; 16],
) -> (ClientWebSocket, ActivationReady) {
    let url = format!("ws://127.0.0.1:{}/ws/id", port + 2);
    for _ in 0..100 {
        let Ok((mut websocket, _)) = connect_async(url.clone()).await else {
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
        let Ok(response) = receive_protocol(&mut websocket, 1_000).await else {
            continue;
        };
        if !matches!(
            response.union,
            Some(rendezvous_message::Union::RegisterPeerResponse(response)) if response.request_pk
        ) {
            continue;
        }
        let mut register_pk = RendezvousMessage::new();
        register_pk.set_register_pk(RegisterPk {
            id: id.to_owned(),
            uuid: Bytes::copy_from_slice(uuid),
            pk: Bytes::copy_from_slice(public_key),
            activation_epoch: epoch,
            activation_id: Bytes::copy_from_slice(&activation_id),
            ..Default::default()
        });
        websocket
            .send(Message::Binary(
                register_pk.write_to_bytes().unwrap().into(),
            ))
            .await
            .unwrap();
        let response = match receive_protocol(&mut websocket, 3_000).await.unwrap().union {
            Some(rendezvous_message::Union::RegisterPkResponse(response)) => response,
            other => panic!("expected WSS Profile Ready ACK, got {other:?}"),
        };
        assert_eq!(
            response.result.enum_value().ok(),
            Some(register_pk_response::Result::OK)
        );
        assert_eq!(response.activation_epoch, epoch);
        assert_eq!(response.activation_id.as_ref(), activation_id);
        assert_eq!(response.route_lease.len(), 32);
        assert!(response.route_generation > 0);
        return (
            websocket,
            ActivationReady {
                epoch,
                activation_id: response.activation_id,
                route_lease: response.route_lease,
                route_generation: response.route_generation,
            },
        );
    }
    panic!("HBBS did not accept Profile Activation WSS registration for {id}");
}

async fn deactivate_websocket(
    id: &str,
    uuid: &[u8; 16],
    websocket: &mut ClientWebSocket,
    ready: &ActivationReady,
) -> DeactivatePeerResponse {
    websocket
        .send(Message::Binary(
            deactivate_message(id, uuid, ready)
                .write_to_bytes()
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    deactivation_result(receive_protocol(websocket, 3_000).await.unwrap())
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
    let mut last_result = "no RegisterPkResponse".to_owned();
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
                last_result = format!("{:?}", response.result);
                if response.result.enum_value().ok() == Some(register_pk_response::Result::OK) {
                    assert!(response.keep_alive > 0);
                    return websocket;
                }
            }
        }
        let _ = websocket.close(None).await;
        sleep(Duration::from_millis(100)).await;
    }
    panic!("HBBS did not accept WebSocket registration for {id}: {last_result}");
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
            assert!(
                punch.relay_quality_offer.is_none(),
                "official/non-opting clients must not receive a Relay Quality offer"
            );
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
            assert_eq!(response.relay_server, relay);
            assert!(response.relay_quality_decision.is_none());
        }
        other => panic!("expected RelayResponse, got {other:?}"),
    }
}

fn successful_report(
    offer: &RelayQualityOffer,
    base_rtt_ms: u32,
    endpoint_role: u32,
) -> RelayProbeReport {
    RelayProbeReport {
        protocol_version: offer.protocol_version,
        allocation_id: offer.allocation_id.clone(),
        results: offer
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| RelayProbeResult {
                relay_server: candidate.relay_server.clone(),
                attempted: offer.probe_samples,
                succeeded: offer.probe_samples,
                rtt_ms: base_rtt_ms.saturating_add(index as u32 * 10),
                jitter_ms: 2,
                ..Default::default()
            })
            .collect(),
        stage: offer.stage,
        stage_token: offer.stage_token.clone(),
        endpoint_role,
        ..Default::default()
    }
}

fn failed_report(offer: &RelayQualityOffer, endpoint_role: u32) -> RelayProbeReport {
    RelayProbeReport {
        protocol_version: offer.protocol_version,
        allocation_id: offer.allocation_id.clone(),
        results: offer
            .candidates
            .iter()
            .map(|candidate| RelayProbeResult {
                relay_server: candidate.relay_server.clone(),
                attempted: offer.probe_samples,
                succeeded: 0,
                rtt_ms: 0,
                jitter_ms: 0,
                ..Default::default()
            })
            .collect(),
        stage: offer.stage,
        stage_token: offer.stage_token.clone(),
        endpoint_role,
        ..Default::default()
    }
}

fn expect_fast_request(
    message: RendezvousMessage,
    session_uuid: &str,
    server_public: &sign::PublicKey,
) -> RequestRelay {
    let request = match message.union {
        Some(rendezvous_message::Union::RequestRelay(request)) => request,
        other => panic!("expected RequestRelay, got {other:?}"),
    };
    assert_eq!(request.uuid, session_uuid);
    assert!(!request.relay_server.is_empty());
    assert!(request.relay_quality_decision.is_some());
    assert_eq!(
        request.relay_server, request.relay_quality_decision.relay_server,
        "ordinary relay_server and RelayQualityDecision must be atomic and identical"
    );
    assert!(request.relay_quality_decision.reason.is_empty());
    assert!(matches!(request.relay_quality_decision.reason_code, 1..=4));
    assert!(!request.fast_relay_authorization.is_empty());

    let payload = sign::verify(request.fast_relay_authorization.as_ref(), server_public)
        .expect("HBBS FastCompat authorization must verify with its public key");
    let authorization = FastRelayAuthorization::parse_from_bytes(&payload).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(authorization.version, 1);
    assert_eq!(authorization.session_uuid, session_uuid);
    assert!(authorization.expires_at >= now.saturating_add(30));
    assert!(authorization.expires_at <= now.saturating_add(300));
    assert!(authorization.allow_fast_compat);
    assert!(!authorization.allow_fast_media_v1);
    assert_eq!(authorization.max_bitrate_kbps, 50_000);
    request
}

async fn send_fast_response(
    target: &mut ClientWebSocket,
    request: &RequestRelay,
    untrusted_authorization: &[u8],
) {
    let mut message = RendezvousMessage::new();
    message.set_relay_response(RelayResponse {
        socket_addr: request.socket_addr.clone(),
        uuid: request.uuid.clone(),
        relay_server: request.relay_server.clone(),
        fast_relay_authorization: Bytes::copy_from_slice(untrusted_authorization),
        ..Default::default()
    });
    target
        .send(Message::Binary(message.write_to_bytes().unwrap().into()))
        .await
        .unwrap();
}

fn expect_fast_response(
    message: RendezvousMessage,
    expected_authorization: &Bytes,
) -> RelayResponse {
    let response = match message.union {
        Some(rendezvous_message::Union::RelayResponse(response)) => response,
        other => panic!("expected RelayResponse, got {other:?}"),
    };
    assert!(response.relay_quality_decision.is_some());
    assert_eq!(
        response.relay_server,
        response.relay_quality_decision.relay_server
    );
    assert_eq!(response.fast_relay_authorization, *expected_authorization);
    response
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
    reserve_hbbs_ports_excluding(0)
}

fn reserve_hbbs_ports_excluding(excluded: u16) -> u16 {
    for _ in 0..2_500 {
        let candidate = NEXT_HBBS_PORT.fetch_add(4, Ordering::SeqCst);
        if candidate > 39_997 {
            break;
        }
        let port = candidate as u16;
        if port.abs_diff(excluded) <= 3 {
            continue;
        }
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
    for _ in 0..6_666 {
        let candidate = NEXT_HBBR_PORT.fetch_add(3, Ordering::SeqCst);
        if candidate > 59_997 {
            break;
        }
        let port = candidate as u16;
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
    wait_for_wss_probe_count(probe_count, 1).await;
}

async fn wait_for_wss_probe_count(probe_count: &AtomicUsize, minimum: usize) {
    for _ in 0..750 {
        if probe_count.load(Ordering::SeqCst) >= minimum {
            // Let HBBS record the successful probe after its close frame.
            sleep(Duration::from_millis(500)).await;
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("HBBS completed fewer than {minimum} certificate-valid /ws/relay probes");
}
