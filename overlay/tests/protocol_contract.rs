use hbb_common::{
    protobuf::{Message as _, MessageField},
    rendezvous_proto::{
        punch_hole_response, rendezvous_message, DeactivatePeer, FastRelayAuthorization, NatType,
        PunchHoleRequest, PunchHoleResponse, RegisterPeer, RegisterPk, RegisterPkResponse,
        RelayProbeReport, RelayProbeRequest, RelayProbeResponse, RelayProbeResult,
        RelayQualityCancel, RelayQualityCandidate, RelayQualityDecision, RelayQualityOffer,
        RelayQualityScore, RelayResponse, RendezvousMessage, RequestRelay,
    },
};

const DENIAL_TEXT: &str = "connection authorization failed";

fn fixture(name: &str) -> Vec<u8> {
    let value = match name {
        "punch-hole-request" => {
            include_str!("../contracts/auth/v1/fixtures/punch-hole-request.hex")
        }
        "request-relay" => include_str!("../contracts/auth/v1/fixtures/request-relay.hex"),
        "punch-hole-denied" => include_str!("../contracts/auth/v1/fixtures/punch-hole-denied.hex"),
        "request-relay-denied" => {
            include_str!("../contracts/auth/v1/fixtures/request-relay-denied.hex")
        }
        _ => panic!("unknown protocol fixture"),
    };
    let value = value.trim().as_bytes();
    assert_eq!(value.len() % 2, 0);
    value
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

#[test]
fn punch_hole_request_fixture_carries_the_controller_token() {
    let request = PunchHoleRequest::parse_from_bytes(&fixture("punch-hole-request")).unwrap();
    assert_eq!(request.id, "123456789");
    assert_eq!(request.nat_type.enum_value().unwrap(), NatType::SYMMETRIC);
    assert_eq!(request.token, "fixture-controller-token");
    assert_eq!(request.version, "1.4.9");
}

#[test]
fn direct_request_relay_fixture_carries_the_same_controller_token() {
    let request = RequestRelay::parse_from_bytes(&fixture("request-relay")).unwrap();
    assert_eq!(request.id, "123456789");
    assert_eq!(request.uuid, "fixture-relay-uuid");
    assert!(request.secure);
    assert_eq!(request.token, "fixture-controller-token");
}

#[test]
fn existing_response_fields_encode_stable_non_enumerating_denials() {
    let punch = PunchHoleResponse::parse_from_bytes(&fixture("punch-hole-denied")).unwrap();
    assert_eq!(
        punch.failure.enum_value().unwrap(),
        punch_hole_response::Failure::OFFLINE
    );
    assert_eq!(punch.other_failure, DENIAL_TEXT);

    let relay = RelayResponse::parse_from_bytes(&fixture("request-relay-denied")).unwrap();
    assert_eq!(relay.refuse_reason, DENIAL_TEXT);
    assert_eq!(relay.version, "1.4.9");
}

#[test]
fn legacy_fixtures_default_all_starry_quality_fields() {
    let punch = PunchHoleRequest::parse_from_bytes(&fixture("punch-hole-request")).unwrap();
    assert_eq!(punch.relay_quality_protocol, 0);
    let request = RequestRelay::parse_from_bytes(&fixture("request-relay")).unwrap();
    assert!(request.relay_quality_report.is_none());
    assert!(request.relay_quality_decision.is_none());
    assert!(request.relay_quality_allocation_id.is_empty());
    assert!(request.fast_relay_authorization.is_empty());
    let response = RelayResponse::parse_from_bytes(&fixture("request-relay-denied")).unwrap();
    assert!(response.relay_quality_report.is_none());
    assert!(response.relay_quality_decision.is_none());
    assert!(response.fast_relay_authorization.is_empty());
}

#[test]
fn legacy_registration_messages_default_profile_activation_fields() {
    let register_peer = RegisterPeer::parse_from_bytes(
        &RegisterPeer {
            id: "123456789".to_owned(),
            serial: 1,
            ..Default::default()
        }
        .write_to_bytes()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(register_peer.route_generation, 0);
    assert_eq!(register_peer.activation_epoch, 0);
    assert!(register_peer.activation_id.is_empty());
    assert!(register_peer.route_lease.is_empty());

    let ready = RegisterPkResponse::default();
    assert_eq!(ready.route_generation, 0);
    assert_eq!(ready.activation_epoch, 0);
    assert!(ready.activation_id.is_empty());
    assert!(ready.route_lease.is_empty());
}

#[test]
fn profile_activation_fields_and_deactivation_envelope_round_trip() {
    let activation_id = vec![0x31; 16];
    let route_lease = vec![0x52; 32];
    let register_peer = RegisterPeer {
        id: "123456789".to_owned(),
        route_generation: 77,
        activation_epoch: 41,
        activation_id: activation_id.clone().into(),
        route_lease: route_lease.clone().into(),
        ..Default::default()
    };
    let register_peer =
        RegisterPeer::parse_from_bytes(&register_peer.write_to_bytes().unwrap()).unwrap();
    assert_eq!(register_peer.route_generation, 77);
    assert_eq!(register_peer.activation_epoch, 41);
    assert_eq!(
        register_peer.activation_id.as_ref(),
        activation_id.as_slice()
    );
    assert_eq!(register_peer.route_lease.as_ref(), route_lease.as_slice());

    let register_pk = RegisterPk {
        id: "123456789".to_owned(),
        uuid: vec![0x75; 16].into(),
        pk: vec![0x70; 32].into(),
        activation_epoch: 41,
        activation_id: activation_id.clone().into(),
        ..Default::default()
    };
    let register_pk = RegisterPk::parse_from_bytes(&register_pk.write_to_bytes().unwrap()).unwrap();
    assert_eq!(register_pk.activation_epoch, 41);
    assert_eq!(register_pk.activation_id.as_ref(), activation_id.as_slice());

    let ready = RegisterPkResponse {
        route_generation: 77,
        activation_epoch: 41,
        activation_id: activation_id.clone().into(),
        route_lease: route_lease.clone().into(),
        ..Default::default()
    };
    let ready = RegisterPkResponse::parse_from_bytes(&ready.write_to_bytes().unwrap()).unwrap();
    assert_eq!(ready.route_generation, 77);
    assert_eq!(ready.activation_epoch, 41);
    assert_eq!(ready.activation_id.as_ref(), activation_id.as_slice());
    assert_eq!(ready.route_lease.as_ref(), route_lease.as_slice());

    let mut envelope = RendezvousMessage::new();
    envelope.set_deactivate_peer(DeactivatePeer {
        id: "123456789".to_owned(),
        network_identity_uuid: vec![0x75; 16].into(),
        activation_epoch: 41,
        activation_id: activation_id.into(),
        route_lease: route_lease.into(),
        route_generation: 77,
        ..Default::default()
    });
    let envelope =
        RendezvousMessage::parse_from_bytes(&envelope.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::DeactivatePeer(deactivate)) = envelope.union else {
        panic!("peer deactivation oneof was not preserved");
    };
    assert_eq!(deactivate.activation_epoch, 41);
    assert_eq!(deactivate.route_generation, 77);
}

#[test]
fn fast_relay_grant_is_identical_on_both_additive_signalling_fields() {
    sodiumoxide::init().unwrap();
    let (public, secret) = sodiumoxide::crypto::sign::gen_keypair();
    let payload = FastRelayAuthorization {
        version: 1,
        session_uuid: "fast-session-1".to_owned(),
        expires_at: 1_800_000_090,
        allow_fast_compat: true,
        allow_fast_media_v1: false,
        max_bitrate_kbps: 50_000,
        ..Default::default()
    }
    .write_to_bytes()
    .unwrap();
    let signed = sodiumoxide::crypto::sign::sign(&payload, &secret);

    let request = RequestRelay {
        id: "123456789".to_owned(),
        uuid: "fast-session-1".to_owned(),
        relay_server: "relay-a.example:21117".to_owned(),
        fast_relay_authorization: signed.clone().into(),
        ..Default::default()
    };
    let response = RelayResponse {
        uuid: "fast-session-1".to_owned(),
        relay_server: "relay-a.example:21117".to_owned(),
        fast_relay_authorization: signed.clone().into(),
        ..Default::default()
    };
    let request = RequestRelay::parse_from_bytes(&request.write_to_bytes().unwrap()).unwrap();
    let response = RelayResponse::parse_from_bytes(&response.write_to_bytes().unwrap()).unwrap();
    assert_eq!(
        request.fast_relay_authorization,
        response.fast_relay_authorization
    );
    assert_eq!(request.relay_server, "relay-a.example:21117");
    assert_eq!(response.relay_server, "relay-a.example:21117");
    let verified =
        sodiumoxide::crypto::sign::verify(request.fast_relay_authorization.as_ref(), &public)
            .unwrap();
    let grant = FastRelayAuthorization::parse_from_bytes(&verified).unwrap();
    assert_eq!(grant.session_uuid, "fast-session-1");
    assert!(grant.allow_fast_compat);
    assert!(!grant.allow_fast_media_v1);
    assert_eq!(grant.max_bitrate_kbps, 50_000);
}

#[test]
fn fast_media_role_grants_round_trip_tags_seven_through_twelve() {
    let allocation_id = vec![0xa5; 16];
    let common = |role| FastRelayAuthorization {
        version: 1,
        session_uuid: "fast-media-session-1".to_owned(),
        expires_at: 1_800_000_090,
        allow_fast_compat: true,
        allow_fast_media_v1: true,
        max_bitrate_kbps: 50_000,
        relay_udp_protocol: 1,
        relay_server: "relay-a.example:21117".to_owned(),
        relay_udp_port: 21_119,
        relay_allocation_id: allocation_id.clone().into(),
        relay_max_datagram: 1_200,
        relay_endpoint_role: role,
        ..Default::default()
    };
    let controller_bytes = common(1).write_to_bytes().unwrap();
    let target_bytes = common(2).write_to_bytes().unwrap();
    assert_ne!(controller_bytes, target_bytes);

    for (bytes, role) in [(controller_bytes, 1), (target_bytes, 2)] {
        let grant = FastRelayAuthorization::parse_from_bytes(&bytes).unwrap();
        assert_eq!(grant.relay_udp_protocol, 1);
        assert_eq!(grant.relay_server, "relay-a.example:21117");
        assert_eq!(grant.relay_udp_port, 21_119);
        assert_eq!(grant.relay_allocation_id.as_ref(), allocation_id.as_slice());
        assert_eq!(grant.relay_max_datagram, 1_200);
        assert_eq!(grant.relay_endpoint_role, role);
    }
}

#[test]
fn enhanced_offer_report_and_decision_round_trip_without_touching_legacy_fields() {
    let allocation_id = vec![0x41; 16];
    let stage_token = vec![0x42; 16];
    let offer = RelayQualityOffer {
        protocol_version: 1,
        allocation_id: allocation_id.clone().into(),
        fallback_relay: "relay-a.example:21117".to_owned(),
        candidates: vec![RelayQualityCandidate {
            relay_server: "relay-a.example:21117".to_owned(),
            probe_url: "wss://relay-a.example/ws/relay".to_owned(),
            ..Default::default()
        }],
        probe_samples: 3,
        probe_interval_ms: 50,
        report_timeout_ms: 15_000,
        probe_timeout_ms: 1_000,
        strategy: 1,
        stage: 1,
        stage_token: stage_token.clone().into(),
        stage_deadline_unix_ms: 1_787_990_407_300,
        total_deadline_unix_ms: 1_787_990_415_000,
        primary_relay: "relay-a.example:21117".to_owned(),
        p2p_probe_grace_ms: 300,
        ..Default::default()
    };
    let report = RelayProbeReport {
        protocol_version: 1,
        allocation_id: allocation_id.clone().into(),
        results: vec![RelayProbeResult {
            relay_server: "relay-a.example:21117".to_owned(),
            attempted: 3,
            succeeded: 3,
            rtt_ms: 44,
            jitter_ms: 3,
            ..Default::default()
        }],
        stage: 1,
        stage_token: stage_token.into(),
        endpoint_role: 2,
        ..Default::default()
    };
    let decision = RelayQualityDecision {
        protocol_version: 1,
        allocation_id: allocation_id.into(),
        relay_server: "relay-a.example:21117".to_owned(),
        scores: vec![RelayQualityScore {
            relay_server: "relay-a.example:21117".to_owned(),
            score: 8_800,
            ..Default::default()
        }],
        reason: String::new(),
        reason_code: 1,
        stage: 1,
        ..Default::default()
    };
    let mut response = PunchHoleResponse {
        relay_server: "relay-a.example:21117".to_owned(),
        relay_quality_offer: MessageField::some(offer),
        relay_quality_peer_report: MessageField::some(report),
        relay_quality_decision: MessageField::some(decision),
        ..Default::default()
    };
    response.set_nat_type(NatType::SYMMETRIC);
    let decoded = PunchHoleResponse::parse_from_bytes(&response.write_to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.relay_server, "relay-a.example:21117");
    assert_eq!(decoded.nat_type(), NatType::SYMMETRIC);
    assert_eq!(decoded.relay_quality_offer.candidates.len(), 1);
    assert_eq!(decoded.relay_quality_offer.strategy, 1);
    assert_eq!(decoded.relay_quality_offer.stage, 1);
    assert!(decoded.relay_quality_offer.weights.is_none());
    assert!(decoded.relay_quality_offer.candidates[0].load.is_none());
    assert_eq!(decoded.relay_quality_peer_report.results[0].rtt_ms, 44);
    assert_eq!(decoded.relay_quality_decision.scores[0].score, 8_800);
    assert_eq!(decoded.relay_quality_decision.reason_code, 1);
    assert!(decoded.relay_quality_decision.reason.is_empty());
}

#[test]
fn staged_offer_report_decision_and_cancel_oneofs_round_trip() {
    let allocation_id = vec![0x51; 16];
    let stage_token = vec![0x52; 16];

    let mut offer_message = RendezvousMessage::new();
    offer_message.set_relay_quality_stage_offer(RelayQualityOffer {
        protocol_version: 1,
        allocation_id: allocation_id.clone().into(),
        candidates: vec![RelayQualityCandidate {
            relay_server: "relay-b.example:21117".to_owned(),
            probe_url: "tcp://relay-b.example:21117".to_owned(),
            ..Default::default()
        }],
        strategy: 1,
        stage: 2,
        stage_token: stage_token.clone().into(),
        ..Default::default()
    });
    let decoded =
        RendezvousMessage::parse_from_bytes(&offer_message.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayQualityStageOffer(offer)) = decoded.union else {
        panic!("staged offer oneof was not preserved");
    };
    assert_eq!(offer.stage, 2);

    let mut report_message = RendezvousMessage::new();
    report_message.set_relay_quality_stage_report(RelayProbeReport {
        protocol_version: 1,
        allocation_id: allocation_id.clone().into(),
        stage: 2,
        stage_token: stage_token.clone().into(),
        endpoint_role: 1,
        ..Default::default()
    });
    let decoded =
        RendezvousMessage::parse_from_bytes(&report_message.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayQualityStageReport(report)) = decoded.union else {
        panic!("staged report oneof was not preserved");
    };
    assert_eq!(report.endpoint_role, 1);

    let mut decision_message = RendezvousMessage::new();
    decision_message.set_relay_quality_stage_decision(RelayQualityDecision {
        protocol_version: 1,
        allocation_id: allocation_id.clone().into(),
        relay_server: "relay-b.example:21117".to_owned(),
        reason_code: 2,
        stage: 2,
        ..Default::default()
    });
    let decoded =
        RendezvousMessage::parse_from_bytes(&decision_message.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayQualityStageDecision(decision)) = decoded.union else {
        panic!("staged decision oneof was not preserved");
    };
    assert_eq!(decision.reason_code, 2);

    let mut cancel_message = RendezvousMessage::new();
    cancel_message.set_relay_quality_cancel(RelayQualityCancel {
        protocol_version: 1,
        allocation_id: allocation_id.into(),
        stage: 2,
        stage_token: stage_token.into(),
        reason_code: 1,
        endpoint_role: 1,
        ..Default::default()
    });
    let decoded =
        RendezvousMessage::parse_from_bytes(&cancel_message.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayQualityCancel(cancel)) = decoded.union else {
        panic!("quality cancel oneof was not preserved");
    };
    assert_eq!(cancel.reason_code, 1);
}

#[test]
fn active_probe_envelope_round_trips_nonce_and_capabilities_without_load() {
    let nonce = vec![0x5a; 16];
    let mut request = RendezvousMessage::new();
    request.set_relay_probe_request(RelayProbeRequest {
        protocol_version: 1,
        nonce: nonce.clone().into(),
        ..Default::default()
    });
    let request = RendezvousMessage::parse_from_bytes(&request.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayProbeRequest(request)) = request.union else {
        panic!("probe request oneof was not preserved");
    };
    assert_eq!(request.nonce.as_ref(), nonce.as_slice());

    let mut response = RendezvousMessage::new();
    response.set_relay_probe_response(RelayProbeResponse {
        protocol_version: 1,
        nonce: nonce.clone().into(),
        starry_version: "1.1.16-patch-v1.3.0".to_owned(),
        relay_probe_protocol: 1,
        relay_load_protocol: 1,
        ..Default::default()
    });
    let response =
        RendezvousMessage::parse_from_bytes(&response.write_to_bytes().unwrap()).unwrap();
    let Some(rendezvous_message::Union::RelayProbeResponse(response)) = response.union else {
        panic!("probe response oneof was not preserved");
    };
    assert_eq!(response.nonce.as_ref(), nonce.as_slice());
    assert!(response.load.is_none());
    assert_eq!(response.relay_probe_protocol, 1);
    assert_eq!(response.relay_load_protocol, 1);

    let legacy = RelayProbeResponse {
        protocol_version: 1,
        nonce: nonce.into(),
        starry_version: "legacy-version-string-must-not-imply-capability".to_owned(),
        ..Default::default()
    };
    let legacy = RelayProbeResponse::parse_from_bytes(&legacy.write_to_bytes().unwrap()).unwrap();
    assert_eq!(legacy.relay_probe_protocol, 0);
    assert_eq!(legacy.relay_load_protocol, 0);
}

#[test]
fn deterministic_protobuf_mutation_corpus_never_panics() {
    let seeds = [
        fixture("punch-hole-request"),
        fixture("request-relay"),
        fixture("punch-hole-denied"),
        fixture("request-relay-denied"),
    ];
    for seed in &seeds {
        for end in 0..seed.len() {
            parse_all_protocol_shapes(&seed[..end]);
        }
        for index in 0..seed.len() {
            for value in [0_u8, 0x01, 0x7f, 0x80, 0xff] {
                let mut mutated = seed.clone();
                mutated[index] = value;
                parse_all_protocol_shapes(&mutated);
            }
        }
    }

    let mut state = 0xbb67_ae85_84ca_a73b_u64;
    for length in [0, 1, 2, 3, 7, 31, 255, 1_024, 8_192, 65_536] {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push(state as u8);
        }
        parse_all_protocol_shapes(&bytes);
    }
}

fn parse_all_protocol_shapes(bytes: &[u8]) {
    let _ = RendezvousMessage::parse_from_bytes(bytes);
    let _ = PunchHoleRequest::parse_from_bytes(bytes);
    let _ = RequestRelay::parse_from_bytes(bytes);
    let _ = PunchHoleResponse::parse_from_bytes(bytes);
    let _ = RelayResponse::parse_from_bytes(bytes);
    let _ = RelayProbeRequest::parse_from_bytes(bytes);
    let _ = RelayProbeResponse::parse_from_bytes(bytes);
    let _ = RelayProbeReport::parse_from_bytes(bytes);
    let _ = RelayQualityOffer::parse_from_bytes(bytes);
    let _ = RelayQualityDecision::parse_from_bytes(bytes);
    let _ = RelayQualityCancel::parse_from_bytes(bytes);
    let _ = FastRelayAuthorization::parse_from_bytes(bytes);
    let _ = RegisterPeer::parse_from_bytes(bytes);
    let _ = RegisterPk::parse_from_bytes(bytes);
    let _ = RegisterPkResponse::parse_from_bytes(bytes);
    let _ = DeactivatePeer::parse_from_bytes(bytes);
}
