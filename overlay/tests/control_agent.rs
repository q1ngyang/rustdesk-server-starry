use base64::{encode_config, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use hbb_common::{
    timeout,
    tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        runtime::Builder,
        time::{sleep, Duration},
    },
};
use rcgen::{
    BasicConstraints, Certificate as GeneratedCertificate, CertificateParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, SanType, PKCS_ECDSA_P256_SHA256,
};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
    ClientConfig, RootCertStore,
};
use serde_json::{json, Value};
use sodiumoxide::crypto::sign;
use std::{
    collections::HashMap,
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_rustls::TlsConnector;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const CLIENT_URI: &str = "spiffe://kessoku.example/control-agent-test";
const WRONG_CLIENT_URI: &str = "spiffe://untrusted.example/control-agent-test";
const ISSUER: &str = "https://kessoku.example.test";
const REQUEST_ID: &str = "018f47d2-4ab0-7def-8b51-2a7d23b82910";
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const ALL_SCOPES: &str = "starry.control.read starry.peer.verify starry.relay.read starry.relay.simulate starry.config.read starry.config.validate starry.config.plan starry.config.apply starry.config.rollback starry.runtime.reload";
const CONFIG_A: &str = "version: 3\nrelay_servers:\n  - 192.0.2.10:21117\n";
const CONFIG_B: &str = "version: 3\nrelay_servers:\n  - 192.0.2.20:21117\n";
const CONFIG_REJECTED: &str = r#"version: 3
relay_servers:
  - relay-rejected.example.test:21117
connection_auth:
  mode: audit
  issuer: https://api.example.test
  audience: rustdesk-connect
  jwks:
    file: missing-jwks.json
"#;
static CONTROL_AGENT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Environment {
    agent: Child,
    hbbs: Child,
    root: PathBuf,
}

impl Drop for Environment {
    fn drop(&mut self) {
        let _ = self.agent.kill();
        let _ = self.agent.wait();
        let _ = self.hbbs.kill();
        let _ = self.hbbs.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct TestIdentity {
    ca_der: Vec<u8>,
    client_der: Vec<u8>,
    client_key: Vec<u8>,
    wrong_client_der: Vec<u8>,
    wrong_client_key: Vec<u8>,
    untrusted_client_der: Vec<u8>,
    untrusted_client_key: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Value,
}

#[test]
fn control_agent_enforces_dual_auth_and_atomic_config_transactions() {
    let _guard = CONTROL_AGENT_TEST_LOCK.lock().unwrap();
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let agent_port = reserve_tcp_port();
            let root = std::env::temp_dir().join(format!(
                "starry-control-agent-{}-{hbbs_port}-{agent_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("tls")).unwrap();
            fs::create_dir_all(root.join("control-auth")).unwrap();
            write_managed_config(&root, CONFIG_A.as_bytes());
            write_local_control_token(&root);

            let instance_id = uuid::Uuid::now_v7().to_string();
            fs::write(root.join("instance-id"), format!("{instance_id}\n")).unwrap();
            let identity = write_tls_identity(&root);
            let (service_public, service_secret) = sign::gen_keypair();
            fs::write(
                root.join("control-auth/kessoku-control.key"),
                encode_config(&service_secret.0, STANDARD_NO_PAD),
            )
            .unwrap();
            fs::write(
                root.join("control-auth/jwks.json"),
                serde_json::to_vec_pretty(&json!({
                    "keys": [{
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "use": "sig",
                        "alg": "EdDSA",
                        "kid": "control-test-key",
                        "key_ops": ["verify"],
                        "x": encode_config(service_public.0, URL_SAFE_NO_PAD)
                    }]
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(root.join("agent.yaml"), agent_config(hbbs_port, agent_port)).unwrap();

            let hbbs = Command::new(env!("CARGO_BIN_EXE_hbbs"))
                .arg("--port")
                .arg(hbbs_port.to_string())
                .arg(format!(
                    "--starry-config={}",
                    root.join("config.yaml").display()
                ))
                .env("TEST_HBBS", "no")
                .env("RUST_LOG", "warn")
                .env(
                    "STARRY_LOCAL_CONTROL_TOKEN_FILE",
                    root.join("local-control.token"),
                )
                .current_dir(&root)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port - 1))).await;
            let agent = Command::new(env!("CARGO_BIN_EXE_starry-control-agent"))
                .arg(root.join("agent.yaml"))
                .env("RUST_LOG", "warn")
                .env("STARRY_TEST_TRANSACTION_DELAY_MS", "150")
                .current_dir(&root)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let _environment = Environment {
                agent,
                hbbs,
                root: root.clone(),
            };
            let agent_address = SocketAddr::from(([127, 0, 0, 1], agent_port));
            wait_until_listening(agent_address).await;

            let allowed_client = client_config(
                &identity.ca_der,
                Some((&identity.client_der, &identity.client_key)),
            );
            let wrong_client = client_config(
                &identity.ca_der,
                Some((&identity.wrong_client_der, &identity.wrong_client_key)),
            );
            let no_certificate = client_config(&identity.ca_der, None);
            assert!(http_request(
                agent_address,
                no_certificate,
                "GET",
                "/control/v1/capabilities",
                &[],
                None,
            )
            .await
            .is_err());
            let untrusted_client = client_config(
                &identity.ca_der,
                Some((
                    &identity.untrusted_client_der,
                    &identity.untrusted_client_key,
                )),
            );
            assert!(http_request(
                agent_address,
                untrusted_client,
                "GET",
                "/control/v1/capabilities",
                &[],
                None,
            )
            .await
            .is_err());

            let valid_token = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                ALL_SCOPES,
                TokenTime::Active,
                None,
            );
            let denied_certificate = request_json(
                agent_address,
                wrong_client,
                "GET",
                "/control/v1/capabilities",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            assert_problem(&denied_certificate, 403, "CLIENT_CERT_DENIED");

            let missing_token = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                None,
                &[],
                None,
            )
            .await;
            assert_problem(&missing_token, 401, "AUTH_REQUIRED");

            let expired = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                ALL_SCOPES,
                TokenTime::Expired,
                None,
            );
            let expired_response = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&expired),
                &[],
                None,
            )
            .await;
            assert_problem(&expired_response, 401, "TOKEN_EXPIRED");

            let wrong_audience = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                ALL_SCOPES,
                TokenTime::Active,
                Some("urn:starry-control:not-this-instance"),
            );
            let wrong_audience_response = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&wrong_audience),
                &[],
                None,
            )
            .await;
            assert_problem(&wrong_audience_response, 401, "TOKEN_INVALID");

            let wrong_azp = service_token(
                &service_secret,
                &instance_id,
                WRONG_CLIENT_URI,
                ALL_SCOPES,
                TokenTime::Active,
                None,
            );
            let wrong_azp_response = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&wrong_azp),
                &[],
                None,
            )
            .await;
            assert_problem(&wrong_azp_response, 401, "TOKEN_INVALID");

            let insufficient = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                "starry.config.read",
                TokenTime::Active,
                None,
            );
            let insufficient_response = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&insufficient),
                &[],
                None,
            )
            .await;
            assert_problem(&insufficient_response, 403, "SCOPE_DENIED");

            let capabilities = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&valid_token),
                &[("X-Request-ID", REQUEST_ID)],
                None,
            )
            .await;
            assert_eq!(capabilities.status, 200);
            assert_eq!(
                capabilities.headers.get("x-request-id").map(String::as_str),
                Some(REQUEST_ID)
            );
            assert_eq!(capabilities.body["instance"]["id"], instance_id);
            assert_eq!(capabilities.body["capabilities"]["config_schema"], 6);
            assert_eq!(capabilities.body["capabilities"]["relay_reallocation"], 1);
            assert_eq!(capabilities.body["capabilities"]["config_transaction"], 1);
            assert_eq!(capabilities.body["capabilities"]["peer_registry"], 2);
            assert_eq!(capabilities.body["capabilities"]["relay_probe_protocol"], 1);
            assert_eq!(capabilities.body["capabilities"]["relay_load_protocol"], 1);
            assert_eq!(
                capabilities.body["capabilities"]["profile_activation_lease"],
                1
            );

            let relay_read_only = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                "starry.relay.read",
                TokenTime::Active,
                None,
            );
            let relays = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/relays",
                Some(&relay_read_only),
                &[],
                None,
            )
            .await;
            assert_eq!(relays.status, 200);
            assert_eq!(relays.body["profile_activation"]["protocol_version"], 1);
            assert!(relays.body["relays"][0]["capabilities"]["relay_probe_protocol"].is_null());
            for legacy_null in [
                "telemetry_schema",
                "process_instance_id",
                "telemetry_sequence",
                "uptime_seconds",
                "active_sessions",
                "pending_pairs",
                "bandwidth_bps",
                "capacity_sessions",
                "draining",
                "admission_open",
            ] {
                assert!(
                    relays.body["relays"][0]["websocket"][legacy_null].is_null(),
                    "legacy field {legacy_null} must be null: {}",
                    relays.body
                );
            }
            assert!(relays.body["quality"]["offer_skip_reasons"].is_object());
            assert!(relays.body["quality"]["fallback_reasons"].is_object());
            assert!(relays.body["quality"]["relay_selections"].is_object());
            assert_eq!(relays.body["quality"]["protocol_version"], 1);
            assert_eq!(relays.body["quality"]["strategy"], "adaptive");
            for counter in [
                "primary_probes",
                "primary_accepted",
                "expansions_triggered",
                "p2p_cancellations",
                "estimated_probe_attempts_saved",
                "expanded_decisions",
                "stage_timeouts",
            ] {
                assert!(
                    relays.body["quality"][counter].is_number(),
                    "quality counter {counter} missing: {}",
                    relays.body
                );
            }
            assert_eq!(
                relays.body["relays"][0]["websocket"]["stale"].as_bool(),
                Some(true),
                "relay inventory: {}",
                relays.body
            );
            let serialized_inventory = serde_json::to_string(&relays.body).unwrap();
            for forbidden in [
                "allocation_id",
                "session_uuid",
                "nonce",
                "stage_token",
                "raw_report",
                "client_ip",
                "target_ip",
            ] {
                assert!(!serialized_inventory.contains(forbidden));
            }
            let wrong_scope_relays = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/relays",
                Some(&insufficient),
                &[],
                None,
            )
            .await;
            assert_problem(&wrong_scope_relays, 403, "SCOPE_DENIED");
            let relay_scope_cannot_read_config = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config",
                Some(&relay_read_only),
                &[],
                None,
            )
            .await;
            assert_problem(&relay_scope_cannot_read_config, 403, "SCOPE_DENIED");
            assert_eq!(relays.body["profile_activation"]["lease_ttl_seconds"], 45);
            assert_eq!(relays.body["profile_activation"]["burst_limit"], 12);
            assert_eq!(
                relays.body["profile_activation"]["burst_window_seconds"],
                30
            );

            let unknown_peer = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/peers:verify",
                Some(&valid_token),
                &[],
                Some(json!({
                    "id": "301132036",
                    "uuid": base64::encode([1_u8; 16]),
                    "activation_epoch": 1,
                    "activation_id": base64::encode([2_u8; 16]),
                    "route_leases": [base64::encode([3_u8; 32])]
                })),
            )
            .await;
            assert_eq!(unknown_peer.status, 200);
            assert_eq!(unknown_peer.body["instance_id"], instance_id);
            assert_eq!(unknown_peer.body["registered"], false);

            let initial = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            assert_eq!(initial.status, 200);
            assert_eq!(initial.body["drift"], false);
            assert_eq!(initial.body["format"], "yaml");
            assert_eq!(initial.body["document"], CONFIG_A);
            let initial_etag = initial.headers.get("etag").unwrap().clone();
            assert_eq!(initial.body["etag"], initial_etag);
            run_kessoku_provider_e2e(&root, agent_port, &instance_id);
            assert_eq!(
                fs::read_to_string(root.join("config.yaml")).unwrap(),
                CONFIG_A
            );

            let invalid_validation = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:validate",
                Some(&valid_token),
                &[],
                Some(json!({"document": "version: nope\n", "format": "yaml"})),
            )
            .await;
            assert_eq!(invalid_validation.status, 200);
            assert_eq!(invalid_validation.body["valid"], false);
            assert!(!invalid_validation.body["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty());
            assert_eq!(
                fs::read_to_string(root.join("config.yaml")).unwrap(),
                CONFIG_A
            );

            let missing_precondition = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&valid_token),
                &[],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_problem(&missing_precondition, 428, "PRECONDITION_REQUIRED");

            let plan = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&valid_token),
                &[
                    ("If-Match", initial_etag.as_str()),
                    ("traceparent", TRACEPARENT),
                ],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_eq!(plan.status, 200);
            assert_eq!(plan.body["base_etag"], initial_etag);
            assert_eq!(plan.body["impact"]["restart_required"], false);
            assert!(!plan.body["changes"].as_array().unwrap().is_empty());
            let apply_body = json!({
                "plan_id": plan.body["plan_id"],
                "candidate_digest": plan.body["candidate_digest"],
                "comment": "integration apply"
            });

            let missing_idempotency = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &[("If-Match", initial_etag.as_str())],
                Some(apply_body.clone()),
            )
            .await;
            assert_problem(&missing_idempotency, 428, "PRECONDITION_REQUIRED");

            let apply_headers = [
                ("If-Match", initial_etag.as_str()),
                ("Idempotency-Key", "control-agent-apply-0001"),
                ("traceparent", TRACEPARENT),
            ];
            let accepted = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &apply_headers,
                Some(apply_body.clone()),
            )
            .await;
            assert_eq!(accepted.status, 202);
            let apply_id = accepted.body["id"].as_str().unwrap().to_owned();
            let apply_audit_id = accepted.body["audit_id"].as_str().unwrap().to_owned();
            let competing = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &[
                    ("If-Match", initial_etag.as_str()),
                    ("Idempotency-Key", "control-agent-apply-0002"),
                ],
                Some(apply_body.clone()),
            )
            .await;
            assert_problem(&competing, 409, "OPERATION_IN_PROGRESS");
            let applied = wait_for_operation(
                agent_address,
                allowed_client.clone(),
                &valid_token,
                &apply_id,
            )
            .await;
            assert_eq!(applied["state"], "succeeded");
            assert_eq!(applied["audit_id"], apply_audit_id);
            assert_eq!(
                applied["activation_ack"]["source_digest"],
                plan.body["candidate_digest"]
            );
            assert!(applied["activation_ack"]["subsystem_acks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|ack| ack["accepted"] == true));
            assert_eq!(
                fs::read_to_string(root.join("config.yaml")).unwrap(),
                CONFIG_B
            );

            let repeated = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &apply_headers,
                Some(apply_body.clone()),
            )
            .await;
            assert_eq!(repeated.status, 202);
            assert_eq!(repeated.body["id"], apply_id);

            let reused = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &apply_headers,
                Some(json!({
                    "plan_id": plan.body["plan_id"],
                    "candidate_digest": plan.body["candidate_digest"],
                    "comment": "different request"
                })),
            )
            .await;
            assert_problem(&reused, 409, "IDEMPOTENCY_KEY_REUSED");

            let history = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config/history",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            assert_eq!(history.status, 200);
            let baseline = history.body["revisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|revision| revision["result"] == "baseline")
                .expect("the first transaction must retain a rollback baseline");
            let baseline_id = baseline["id"].as_str().unwrap().to_owned();

            let current = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            let current_etag = current.headers.get("etag").unwrap().clone();
            let rollback = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:rollback",
                Some(&valid_token),
                &[
                    ("If-Match", current_etag.as_str()),
                    ("Idempotency-Key", "control-agent-rollback-0001"),
                ],
                Some(json!({"revision_id": baseline_id, "comment": "restore baseline"})),
            )
            .await;
            assert_eq!(rollback.status, 202);
            let rollback_id = rollback.body["id"].as_str().unwrap();
            let rolled_to_baseline = wait_for_operation(
                agent_address,
                allowed_client.clone(),
                &valid_token,
                rollback_id,
            )
            .await;
            assert_eq!(rolled_to_baseline["state"], "succeeded");
            assert_eq!(
                fs::read_to_string(root.join("config.yaml")).unwrap(),
                CONFIG_A
            );

            let drift_base = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            let drift_etag = drift_base.headers.get("etag").unwrap().clone();
            let drift_plan = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&valid_token),
                &[("If-Match", drift_etag.as_str())],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_eq!(drift_plan.status, 200);
            fs::write(
                root.join("config.yaml"),
                format!("# external drift\n{CONFIG_A}"),
            )
            .unwrap();
            let drift_rejected = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &[
                    ("If-Match", drift_etag.as_str()),
                    ("Idempotency-Key", "control-agent-drift-0001"),
                ],
                Some(json!({
                    "plan_id": drift_plan.body["plan_id"],
                    "candidate_digest": drift_plan.body["candidate_digest"]
                })),
            )
            .await;
            assert_problem(&drift_rejected, 412, "CONFIG_ETAG_MISMATCH");
            fs::write(root.join("config.yaml"), CONFIG_A).unwrap();

            let before_failure = request_json(
                agent_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/config",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            let before_failure_etag = before_failure.headers.get("etag").unwrap().clone();
            let expected_source_digest = before_failure.body["source_digest"]
                .as_str()
                .unwrap()
                .to_owned();
            let reload_headers = [
                ("Idempotency-Key", "control-agent-reload-0001"),
                ("traceparent", TRACEPARENT),
            ];
            let reloaded = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/runtime:reload",
                Some(&valid_token),
                &reload_headers,
                Some(json!({"expected_source_digest": expected_source_digest.clone()})),
            )
            .await;
            assert_eq!(reloaded.status, 200);
            assert_eq!(reloaded.body["source_digest"], expected_source_digest);
            let reload_audit_id = reloaded.body["audit_id"].as_str().unwrap();
            let reload_audit: Value = serde_json::from_slice(
                &fs::read(
                    root.join("history/audit")
                        .join(format!("{reload_audit_id}.json")),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(reload_audit["action"], "runtime_reload");
            assert_eq!(reload_audit["traceparent"], TRACEPARENT);
            let repeated_reload = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/runtime:reload",
                Some(&valid_token),
                &reload_headers,
                Some(json!({"expected_source_digest": expected_source_digest.clone()})),
            )
            .await;
            assert_eq!(repeated_reload.status, 200);
            assert_eq!(repeated_reload.body, reloaded.body);
            let failure_plan = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&valid_token),
                &[("If-Match", before_failure_etag.as_str())],
                Some(config_candidate(CONFIG_REJECTED)),
            )
            .await;
            assert_eq!(failure_plan.status, 200);
            let failure_apply = request_json(
                agent_address,
                allowed_client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&valid_token),
                &[
                    ("If-Match", before_failure_etag.as_str()),
                    ("Idempotency-Key", "control-agent-apply-failure-0001"),
                ],
                Some(json!({
                    "plan_id": failure_plan.body["plan_id"],
                    "candidate_digest": failure_plan.body["candidate_digest"],
                    "comment": "must roll back"
                })),
            )
            .await;
            assert_eq!(failure_apply.status, 202);
            let failure_id = failure_apply.body["id"].as_str().unwrap();
            let failed = wait_for_operation(
                agent_address,
                allowed_client.clone(),
                &valid_token,
                failure_id,
            )
            .await;
            assert_eq!(failed["state"], "rolled_back");
            assert_eq!(failed["error"]["code"], "CONFIG_INVALID");
            assert_eq!(
                fs::read_to_string(root.join("config.yaml")).unwrap(),
                CONFIG_A
            );
            assert!(root
                .join("history/operations")
                .join(format!("{failure_id}.json"))
                .is_file());
            assert!(root
                .join("history/audit")
                .read_dir()
                .unwrap()
                .next()
                .is_some());
            let apply_audit: Value = serde_json::from_slice(
                &fs::read(
                    root.join("history/audit")
                        .join(format!("{apply_audit_id}.json")),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(apply_audit["traceparent"], TRACEPARENT);
            assert_eq!(apply_audit["request_id"].as_str().unwrap().len(), 36);
            assert_history_excludes_secret(&root.join("history"), &valid_token);
            assert_history_excludes_secret(&root.join("history"), "control-agent-apply-0001");

            let read_only_port = reserve_tcp_port();
            let read_only_config = agent_config(hbbs_port, read_only_port)
                .replace("write_enabled: true", "write_enabled: false")
                .replace("backup_dir: history", "backup_dir: read-only-history");
            fs::write(root.join("read-only-agent.yaml"), read_only_config).unwrap();
            let mut read_only_agent = Command::new(env!("CARGO_BIN_EXE_starry-control-agent"))
                .arg(root.join("read-only-agent.yaml"))
                .env("RUST_LOG", "warn")
                .current_dir(&root)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let read_only_address = SocketAddr::from(([127, 0, 0, 1], read_only_port));
            wait_until_listening(read_only_address).await;
            let read_only_capabilities = request_json(
                read_only_address,
                allowed_client.clone(),
                "GET",
                "/control/v1/capabilities",
                Some(&valid_token),
                &[],
                None,
            )
            .await;
            assert_eq!(read_only_capabilities.status, 200);
            assert!(read_only_capabilities.body["capabilities"]["config_transaction"].is_null());
            let disabled_plan = request_json(
                read_only_address,
                allowed_client,
                "POST",
                "/control/v1/config:plan",
                Some(&valid_token),
                &[("If-Match", before_failure_etag.as_str())],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_problem(&disabled_plan, 404, "REQUEST_INVALID");
            let _ = read_only_agent.kill();
            let _ = read_only_agent.wait();
        });
}

#[test]
fn reload_outage_restores_exact_bytes_and_blocks_writes_until_reconciled() {
    let _guard = CONTROL_AGENT_TEST_LOCK.lock().unwrap();
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            sodiumoxide::init().unwrap();
            let hbbs_port = reserve_hbbs_ports();
            let agent_port = reserve_tcp_port();
            let root = std::env::temp_dir().join(format!(
                "starry-control-agent-recovery-{}-{hbbs_port}-{agent_port}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("tls")).unwrap();
            fs::create_dir_all(root.join("control-auth")).unwrap();
            write_managed_config(&root, CONFIG_A.as_bytes());
            write_local_control_token(&root);
            let instance_id = uuid::Uuid::now_v7().to_string();
            fs::write(root.join("instance-id"), format!("{instance_id}\n")).unwrap();
            let identity = write_tls_identity(&root);
            let (service_public, service_secret) = sign::gen_keypair();
            fs::write(
                root.join("control-auth/jwks.json"),
                serde_json::to_vec(&json!({
                    "keys": [{
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "use": "sig",
                        "alg": "EdDSA",
                        "kid": "control-test-key",
                        "key_ops": ["verify"],
                        "x": encode_config(service_public.0, URL_SAFE_NO_PAD)
                    }]
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(root.join("agent.yaml"), agent_config(hbbs_port, agent_port)).unwrap();

            let hbbs = spawn_hbbs(&root, hbbs_port);
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port - 1))).await;
            let agent = Command::new(env!("CARGO_BIN_EXE_starry-control-agent"))
                .arg(root.join("agent.yaml"))
                .env("RUST_LOG", "warn")
                .env("STARRY_TEST_POST_WRITE_DELAY_MS", "1000")
                .current_dir(&root)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let mut environment = Environment {
                agent,
                hbbs,
                root: root.clone(),
            };
            let agent_address = SocketAddr::from(([127, 0, 0, 1], agent_port));
            wait_until_listening(agent_address).await;
            let client = client_config(
                &identity.ca_der,
                Some((&identity.client_der, &identity.client_key)),
            );
            let token = service_token(
                &service_secret,
                &instance_id,
                CLIENT_URI,
                ALL_SCOPES,
                TokenTime::Active,
                None,
            );

            let initial = request_json(
                agent_address,
                client.clone(),
                "GET",
                "/control/v1/config",
                Some(&token),
                &[],
                None,
            )
            .await;
            let etag = initial.headers.get("etag").unwrap().clone();
            let plan = request_json(
                agent_address,
                client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&token),
                &[("If-Match", etag.as_str())],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_eq!(plan.status, 200);
            let apply = request_json(
                agent_address,
                client.clone(),
                "POST",
                "/control/v1/config:apply",
                Some(&token),
                &[
                    ("If-Match", etag.as_str()),
                    ("Idempotency-Key", "recovery-outage-apply-0001"),
                ],
                Some(json!({
                    "plan_id": plan.body["plan_id"],
                    "candidate_digest": plan.body["candidate_digest"],
                    "comment": "inject HBBS outage after atomic rename"
                })),
            )
            .await;
            assert_eq!(apply.status, 202);
            let operation_id = apply.body["id"].as_str().unwrap().to_owned();
            wait_for_file_bytes(&root.join("config.yaml"), CONFIG_B.as_bytes()).await;
            environment.hbbs.kill().unwrap();
            environment.hbbs.wait().unwrap();

            let operation =
                wait_for_operation(agent_address, client.clone(), &token, &operation_id).await;
            assert_eq!(operation["state"], "manual_intervention_required");
            assert_eq!(operation["error"]["code"], "ROLLBACK_FAILED");
            assert_eq!(
                fs::read(root.join("config.yaml")).unwrap(),
                CONFIG_A.as_bytes()
            );

            environment.hbbs = spawn_hbbs(&root, hbbs_port);
            wait_until_listening(SocketAddr::from(([127, 0, 0, 1], hbbs_port - 1))).await;
            let current = request_json(
                agent_address,
                client.clone(),
                "GET",
                "/control/v1/config",
                Some(&token),
                &[],
                None,
            )
            .await;
            let current_etag = current.headers.get("etag").unwrap().clone();
            let plan = request_json(
                agent_address,
                client.clone(),
                "POST",
                "/control/v1/config:plan",
                Some(&token),
                &[("If-Match", current_etag.as_str())],
                Some(config_candidate(CONFIG_B)),
            )
            .await;
            assert_eq!(plan.status, 200);
            let blocked = request_json(
                agent_address,
                client,
                "POST",
                "/control/v1/config:apply",
                Some(&token),
                &[
                    ("If-Match", current_etag.as_str()),
                    ("Idempotency-Key", "recovery-outage-apply-0002"),
                ],
                Some(json!({
                    "plan_id": plan.body["plan_id"],
                    "candidate_digest": plan.body["candidate_digest"]
                })),
            )
            .await;
            assert_problem(&blocked, 503, "ROLLBACK_FAILED");
        });
}

#[derive(Clone, Copy)]
enum TokenTime {
    Active,
    Expired,
}

fn service_token(
    secret: &sign::SecretKey,
    instance_id: &str,
    azp: &str,
    scopes: &str,
    timing: TokenTime,
    audience: Option<&str>,
) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (iat, nbf, exp) = match timing {
        TokenTime::Active => (now.saturating_sub(1), now.saturating_sub(1), now + 120),
        TokenTime::Expired => (now.saturating_sub(120), now.saturating_sub(120), now - 60),
    };
    let header = encode_config(
        serde_json::to_vec(&json!({
            "alg": "EdDSA",
            "kid": "control-test-key",
            "typ": "JWT"
        }))
        .unwrap(),
        URL_SAFE_NO_PAD,
    );
    let payload = encode_config(
        serde_json::to_vec(&json!({
            "iss": ISSUER,
            "aud": audience.map(str::to_owned).unwrap_or_else(|| format!("urn:starry-control:{instance_id}")),
            "sub": "kessoku-control-service",
            "azp": azp,
            "scope": scopes,
            "act": {"sub": "integration-admin"},
            "iat": iat,
            "nbf": nbf,
            "exp": exp,
            "jti": uuid::Uuid::now_v7().to_string()
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

fn agent_config(hbbs_port: u16, agent_port: u16) -> String {
    format!(
        r#"version: 1
instance_id_file: instance-id
listen: 127.0.0.1:{agent_port}
tls:
  ca_file: tls/ca.pem
  cert_file: tls/server.pem
  key_file: tls/server-key.pem
  allowed_client_uri_sans:
    - {CLIENT_URI}
service_jwt:
  issuer: {ISSUER}
  jwks_file: control-auth/jwks.json
  audience_prefix: "urn:starry-control:"
local_control:
  address: 127.0.0.1:{}
  token_file: local-control.token
config:
  write_enabled: true
  path: config.yaml
  backup_dir: history
  max_bytes: 1048576
"#,
        hbbs_port - 1
    )
}

fn spawn_hbbs(root: &Path, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_hbbs"))
        .arg("--port")
        .arg(port.to_string())
        .arg(format!(
            "--starry-config={}",
            root.join("config.yaml").display()
        ))
        .env("TEST_HBBS", "no")
        .env("RUST_LOG", "warn")
        .env(
            "STARRY_LOCAL_CONTROL_TOKEN_FILE",
            root.join("local-control.token"),
        )
        .current_dir(root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

fn write_local_control_token(root: &Path) {
    let path = root.join("local-control.token");
    fs::write(&path, "localControlTokenForOrdinaryIntegration01\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn write_managed_config(root: &Path, bytes: &[u8]) {
    let path = root.join("config.yaml");
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).unwrap();
}

fn write_tls_identity(root: &Path) -> TestIdentity {
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut ca_name = DistinguishedName::new();
    ca_name.push(DnType::CommonName, "Starry Control integration CA");
    ca_params.distinguished_name = ca_name;
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let server_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server = server_params.signed_by(&server_key, &ca, &ca_key).unwrap();

    let client = client_certificate(CLIENT_URI, &ca, &ca_key);
    let wrong_client = client_certificate(WRONG_CLIENT_URI, &ca, &ca_key);
    let untrusted_ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut untrusted_ca_params = CertificateParams::new(Vec::new()).unwrap();
    untrusted_ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let untrusted_ca = untrusted_ca_params.self_signed(&untrusted_ca_key).unwrap();
    let untrusted_client = client_certificate(CLIENT_URI, &untrusted_ca, &untrusted_ca_key);
    let ca_der = ca.der().to_vec();
    let client_der = client.certificate.der().to_vec();
    let wrong_client_der = wrong_client.certificate.der().to_vec();
    let untrusted_client_der = untrusted_client.certificate.der().to_vec();
    fs::write(root.join("tls/ca.pem"), ca.pem()).unwrap();
    fs::write(root.join("tls/server.pem"), server.pem()).unwrap();
    fs::write(root.join("tls/server-key.pem"), server_key.serialize_pem()).unwrap();
    fs::write(root.join("tls/client.pem"), client.certificate.pem()).unwrap();
    fs::write(root.join("tls/client-key.pem"), client.key.serialize_pem()).unwrap();
    TestIdentity {
        ca_der,
        client_der,
        client_key: client.key.serialize_der(),
        wrong_client_der,
        wrong_client_key: wrong_client.key.serialize_der(),
        untrusted_client_der,
        untrusted_client_key: untrusted_client.key.serialize_der(),
    }
}

fn run_kessoku_provider_e2e(root: &Path, agent_port: u16, instance_id: &str) {
    let Some(binary) = std::env::var_os("KESSOKU_PROVIDER_E2E_BIN") else {
        return;
    };
    let status = Command::new(binary)
        .args([
            "-test.v",
            "-test.run",
            "^TestRealStarryAgentProviderE2E$",
            "-test.count=1",
        ])
        .env(
            "STARRY_E2E_BASE_URL",
            format!("https://localhost:{agent_port}"),
        )
        .env("STARRY_E2E_INSTANCE_ID", instance_id)
        .env("STARRY_E2E_TLS_SERVER_NAME", "localhost")
        .env("STARRY_E2E_CA_FILE", root.join("tls/ca.pem"))
        .env("STARRY_E2E_CLIENT_CERT_FILE", root.join("tls/client.pem"))
        .env(
            "STARRY_E2E_CLIENT_KEY_FILE",
            root.join("tls/client-key.pem"),
        )
        .env(
            "STARRY_E2E_CONTROL_KEY_FILE",
            root.join("control-auth/kessoku-control.key"),
        )
        .env("STARRY_E2E_CONTROL_KEY_ID", "control-test-key")
        .env("STARRY_E2E_CONTROL_ISSUER", ISSUER)
        .env("STARRY_E2E_AUTHORIZED_PARTY", CLIENT_URI)
        .status()
        .expect("start the Kessoku provider E2E binary");
    assert!(status.success(), "Kessoku provider E2E failed: {status}");
}

struct GeneratedIdentity {
    certificate: GeneratedCertificate,
    key: KeyPair,
}

fn client_certificate(
    uri: &str,
    issuer: &GeneratedCertificate,
    issuer_key: &KeyPair,
) -> GeneratedIdentity {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut params = CertificateParams::new(Vec::new()).unwrap();
    params.subject_alt_names = vec![SanType::URI(uri.try_into().unwrap())];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let certificate = params.signed_by(&key, issuer, issuer_key).unwrap();
    GeneratedIdentity { certificate, key }
}

fn client_config(ca_der: &[u8], identity: Option<(&[u8], &[u8])>) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der.to_vec())).unwrap();
    let builder = ClientConfig::builder().with_root_certificates(roots);
    let config = match identity {
        Some((certificate, key)) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from(certificate.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
            )
            .unwrap(),
        None => builder.with_no_client_auth(),
    };
    Arc::new(config)
}

fn config_candidate(document: &str) -> Value {
    json!({"document": document, "format": "yaml"})
}

async fn request_json(
    address: SocketAddr,
    client: Arc<ClientConfig>,
    method: &str,
    path: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> HttpResponse {
    let mut owned_headers = Vec::new();
    if let Some(token) = token {
        owned_headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
    }
    owned_headers.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    let borrowed: Vec<(&str, &str)> = owned_headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    http_request(address, client, method, path, &borrowed, body)
        .await
        .unwrap()
}

async fn http_request(
    address: SocketAddr,
    client: Arc<ClientConfig>,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Result<HttpResponse, String> {
    let stream = TcpStream::connect(address)
        .await
        .map_err(|error| error.to_string())?;
    let connector = TlsConnector::from(client);
    let server_name = ServerName::try_from("localhost").unwrap();
    let mut stream = connector
        .connect(server_name, stream)
        .await
        .map_err(|error| error.to_string())?;
    let body = body
        .map(|value| serde_json::to_vec(&value).unwrap())
        .unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&body)
        .await
        .map_err(|error| error.to_string())?;
    let mut raw = Vec::new();
    timeout(10_000, stream.read_to_end(&mut raw))
        .await
        .map_err(|_| "HTTP response timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    parse_http_response(&raw)
}

fn parse_http_response(raw: &[u8]) -> Result<HttpResponse, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response has no header terminator".to_owned())?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|error| error.to_string())?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "HTTP response has no status".to_owned())?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let body = if raw.len() == split + 4 {
        Value::Null
    } else {
        serde_json::from_slice(&raw[split + 4..]).map_err(|error| error.to_string())?
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn assert_problem(response: &HttpResponse, status: u16, code: &str) {
    assert_eq!(
        response.status, status,
        "unexpected problem: {}",
        response.body
    );
    assert_eq!(response.body["status"], status);
    assert_eq!(response.body["code"], code);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("application/problem+json")
    );
}

async fn wait_for_operation(
    address: SocketAddr,
    client: Arc<ClientConfig>,
    token: &str,
    operation_id: &str,
) -> Value {
    for _ in 0..100 {
        let response = request_json(
            address,
            client.clone(),
            "GET",
            &format!("/control/v1/operations/{operation_id}"),
            Some(token),
            &[],
            None,
        )
        .await;
        assert_eq!(
            response.status, 200,
            "operation lookup failed: {}",
            response.body
        );
        if matches!(
            response.body["state"].as_str(),
            Some("succeeded" | "rolled_back" | "failed" | "manual_intervention_required")
        ) {
            return response.body;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("operation {operation_id} did not complete");
}

async fn wait_for_file_bytes(path: &Path, expected: &[u8]) {
    for _ in 0..200 {
        if fs::read(path).ok().as_deref() == Some(expected) {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{} did not reach the expected transaction bytes",
        path.display()
    );
}

fn assert_history_excludes_secret(root: &Path, secret: &str) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                let raw = fs::read(entry.path()).unwrap();
                assert!(
                    !raw.windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "durable state contains an unredacted secret"
                );
            }
        }
    }
}

async fn wait_until_listening(address: SocketAddr) {
    for _ in 0..200 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("{address} did not start listening");
}

fn reserve_tcp_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn reserve_hbbs_ports() -> u16 {
    for port in 30_001..60_000_u16 {
        let Ok(admin) = StdTcpListener::bind(("127.0.0.1", port - 1)) else {
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
        drop((admin, tcp, udp, websocket));
        return port;
    }
    panic!("could not reserve HBBS ports");
}
