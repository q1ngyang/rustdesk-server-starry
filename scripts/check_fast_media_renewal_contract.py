#!/usr/bin/env python3
"""Validate the frozen patch-v1.3.2 renewal contract without runtime sources."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "contracts/fast-media-renewal/v1"
SUMMARY = ROOT / "contracts/patch-v1.3.2/CONTRACT-RELEASE-SUMMARY.json"


def read_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(value, dict), f"expected JSON object: {path}"
    return value


def digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def proto_field_tags(message: str) -> dict[str, int]:
    return {
        name: int(tag)
        for _, name, tag in re.findall(
            r"^\s*(?:repeated\s+)?([A-Za-z0-9_.]+)\s+([a-z0-9_]+)\s*=\s*([0-9]+);",
            message,
            flags=re.MULTILINE,
        )
    }


def message_block(proto: str, name: str) -> str:
    marker = f"message {name} {{"
    start = proto.index(marker)
    end = proto.index("\n}", start)
    return proto[start : end + 2]


def main() -> None:
    proto = (CONTRACT / "rendezvous-extension.proto").read_text(encoding="utf-8")
    authorization = proto_field_tags(message_block(proto, "FastRelayAuthorization"))
    assert authorization == {
        "version": 1,
        "session_uuid": 2,
        "expires_at": 3,
        "allow_fast_compat": 4,
        "allow_fast_media_v1": 5,
        "max_bitrate_kbps": 6,
        "relay_udp_protocol": 7,
        "relay_server": 8,
        "relay_udp_port": 9,
        "relay_allocation_id": 10,
        "relay_max_datagram": 11,
        "relay_endpoint_role": 12,
        "fast_media_relay_renewal": 13,
        "relay_session_id": 14,
        "renewal_sequence": 15,
        "previous_authorization_sha256": 16,
    }
    request = proto_field_tags(message_block(proto, "FastMediaRenewalRequest"))
    response = proto_field_tags(message_block(proto, "FastMediaRenewalResponse"))
    assert request == {
        "protocol_version": 1,
        "session_uuid": 2,
        "relay_allocation_id": 3,
        "relay_session_id": 4,
        "current_renewal_sequence": 5,
        "controller_authorization_sha256": 6,
        "target_authorization_sha256": 7,
        "request_id": 8,
        "token": 9,
        "relay_server": 10,
        "relay_udp_protocol": 11,
        "relay_max_datagram": 12,
        "current_max_bitrate_kbps": 13,
        "requester_role": 14,
    }
    assert response == {
        "protocol_version": 1,
        "status": 2,
        "session_uuid": 3,
        "relay_allocation_id": 4,
        "relay_session_id": 5,
        "renewal_sequence": 6,
        "expires_at": 7,
        "request_id": 8,
        "controller_authorization": 9,
        "target_authorization": 10,
        "relay_server": 11,
        "relay_udp_protocol": 12,
        "relay_max_datagram": 13,
        "max_bitrate_kbps": 14,
        "renew_after": 15,
        "fallback_before": 16,
    }
    assert "fast_media_renewal_request = 106" in proto
    assert "fast_media_renewal_response = 107" in proto
    for status, number in (
        ("OK", 1),
        ("DISABLED", 2),
        ("UNAUTHENTICATED", 3),
        ("NOT_FOUND", 4),
        ("BINDING_MISMATCH", 5),
        ("EXPIRED", 6),
        ("TOO_EARLY", 7),
        ("RATE_LIMITED", 8),
        ("UNAVAILABLE", 9),
        ("INVALID", 10),
    ):
        assert f"FAST_MEDIA_RENEWAL_STATUS_{status} = {number};" in proto

    semantics = read_json(CONTRACT / "renewal-contract.json")
    assert semantics["status"] == "FROZEN"
    assert semantics["base_contracts"]["preserved_akr1_kinds"] == [1, 2, 3, 4, 5]
    assert semantics["capability"] == {
        "name": "fast_media_relay_renewal",
        "version": 1,
        "source": "authenticated HBBR telemetry consumed by HBBS",
        "version_string_inference_forbidden": True,
    }
    assert semantics["authorization_extension"]["maximum_combined_authorization_bytes"] == 4096
    assert semantics["authorization_extension"]["maximum_future_expiry_seconds"] == 300
    assert semantics["akf1_replay"]["window_packets"] == 2048
    assert semantics["akf1_replay"]["words"] == 32
    assert semantics["akf1_replay"]["maximum_forward_sequence_jump"] == 1_048_576
    assert semantics["lifecycle"]["immortal_allocation_forbidden"] is True
    assert semantics["admission"]["same_nat_roles_are_summed"] is True
    assert semantics["privacy"]["kessoku_media_or_key_path"] is False

    fixture = read_json(CONTRACT / "fixtures/renewal-flow.json")
    assert fixture["request"]["current_renewal_sequence"] == 0
    assert fixture["response"]["renewal_sequence"] == 1
    assert fixture["response"]["status"] == 1
    assert fixture["renewed_controller"]["relay_endpoint_role"] == 1
    assert fixture["renewed_target"]["relay_endpoint_role"] == 2

    telemetry = read_json(ROOT / "contracts/relay-telemetry/v3/telemetry.schema.json")
    telemetry_example = read_json(
        ROOT / "contracts/relay-telemetry/v3/telemetry.example.json"
    )
    assert telemetry["properties"]["telemetry_schema"]["const"] == 3
    fast_media = telemetry["properties"]["fast_media"]
    required = set(fast_media["required"])
    assert {
        "renewal_protocol",
        "replay_window_packets",
        "maximum_forward_sequence_jump",
        "max_session_seconds",
        "reserved_bytes_per_second",
        "renewal_grants_accepted",
        "post_expiry_rebinds",
        "admission_rejected_per_ip",
        "minimum_remaining_ttl_seconds",
    } <= required
    assert set(telemetry_example) == set(telemetry["required"])
    assert set(telemetry_example["fast_media"]) == required
    assert telemetry_example["fast_media"]["renewal_protocol"] == 1
    assert telemetry_example["fast_media"]["replay_window_packets"] == 2048

    capabilities = read_json(
        ROOT / "contracts/control/v1/examples/capabilities.json"
    )
    relays = read_json(ROOT / "contracts/control/v1/examples/relays.json")
    assert capabilities["protocol"]["version"] == "1.1.0"
    assert capabilities["capabilities"]["config_schema"] == 5
    assert capabilities["capabilities"]["relay_telemetry_schema"] == 3
    assert capabilities["capabilities"]["fast_media_relay_udp"] == 1
    assert capabilities["capabilities"]["fast_media_relay_renewal"] == 1
    relay = relays["relays"][0]
    assert relay["capabilities"]["fast_media_relay_renewal"] == 1
    assert relay["websocket"]["telemetry_schema"] == 3
    assert relay["fast_media_udp"]["replay_window_packets"] == 2048
    assert relays["fast_relay"]["renewal_protocol_version"] == 1
    serialized = json.dumps(relays).lower()
    for forbidden in (
        "client_ip",
        "session_uuid",
        "allocation_id",
        "request_id",
        "nonce",
        "raw_report",
        "signed_grant",
    ):
        assert forbidden not in serialized

    summary = read_json(SUMMARY)
    assert summary["status"] == "FROZEN"
    assert summary["source_binding"]["baseline_commit"] == (
        "1b8080bf074e3236cf9a3c0dfae2bdf16832249e"
    )
    assert summary["schema_v5_capability"]["value"] == 5
    assert summary["renewal_capability"]["value"] == 1
    files = summary["files"]
    assert len({item["id"] for item in files}) == len(files)
    assert len({item["path"] for item in files}) == len(files)
    for item in files:
        assert digest(ROOT / item["path"]) == item["sha256"], item["path"]
    inherited = summary["inherited_frozen_contracts"]
    # The patch-v1.3.1 summary records historical bytes at its immutable tag.
    # All other inherited contracts remain byte-identical in this branch.
    for item in inherited:
        if item["id"] != "patch-v1.3.1-contract-candidate":
            assert digest(ROOT / item["path"]) == item["sha256"], item["path"]
    assert all(gate["status"] == "BLOCKED" for gate in summary["preview_release_gates"])
    assert all(gate["status"] == "BLOCKED" for gate in summary["stable_latest_gates"])

    print("FastMedia active-session renewal v1 contract and fixtures are frozen and valid")


if __name__ == "__main__":
    main()
