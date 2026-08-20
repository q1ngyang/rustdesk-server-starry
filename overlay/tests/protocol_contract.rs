use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{
        punch_hole_response, NatType, PunchHoleRequest, PunchHoleResponse, RelayResponse,
        RendezvousMessage, RequestRelay,
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
}
