#!/usr/bin/env python3
"""Fail closed on malformed or accidentally broadened public contracts."""

from __future__ import annotations

import base64
import hashlib
import json
import re
from pathlib import Path

from write_release_summary import build_summary


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts"


def read_json(path: Path) -> object:
    with path.open("r", encoding="utf-8") as stream:
        return json.load(stream)


def decode_jwt_part(value: str) -> object:
    value += "=" * (-len(value) % 4)
    return json.loads(base64.urlsafe_b64decode(value).decode("utf-8"))


def check_json_contracts() -> None:
    for path in sorted(CONTRACTS.rglob("*.json")):
        read_json(path)

    schema = read_json(CONTRACTS / "config/v5/config.schema.json")
    assert isinstance(schema, dict)
    assert schema.get("additionalProperties") is False
    assert schema["properties"]["version"]["enum"] == [1, 2, 3, 4, 5]
    auth = schema["$defs"]["connectionAuth"]
    assert auth["additionalProperties"] is False
    assert auth["properties"]["mode"]["enum"] == ["off", "audit", "enforce"]
    assert auth["properties"]["max_token_bytes"]["maximum"] == 8192
    introspection = schema["$defs"]["introspection"]
    assert introspection["additionalProperties"] is False
    assert any(
        set(rule.get("then", {}).get("required", ()))
        == {"ca_file", "cert_file", "key_file", "server_name"}
        for rule in introspection["allOf"]
    ), "configured introspection must require an mTLS identity"
    quality = schema["$defs"]["relayQuality"]
    assert quality["additionalProperties"] is False
    assert quality["properties"]["max_candidates"]["maximum"] == 5
    assert quality["properties"]["probe_samples"]["maximum"] == 20
    assert quality["properties"]["probe_timeout_ms"]["maximum"] == 5000
    assert quality["properties"]["report_timeout_ms"]["default"] == 15000
    assert quality["properties"]["max_telemetry_age_seconds"]["default"] == 180
    assert quality["properties"]["legacy_fallback_relays"]["uniqueItems"] is True
    assert quality["properties"]["missing_report_penalty_basis_points"]["maximum"] == 10000
    weights = schema["$defs"]["relayQualityWeights"]["properties"]
    assert set(weights) == {"rtt", "jitter", "loss", "load"}

    fast_relay = schema["$defs"]["fastRelay"]
    assert fast_relay["additionalProperties"] is False
    assert fast_relay["properties"]["fast_compat_enabled"]["default"] is False
    assert fast_relay["properties"]["fast_media_v1_enabled"]["default"] is False
    assert fast_relay["properties"]["relay_max_datagram"]["minimum"] == 608
    assert fast_relay["properties"]["relay_max_datagram"]["maximum"] == 1400

    telemetry = read_json(CONTRACTS / "relay-telemetry/v2/telemetry.schema.json")
    assert isinstance(telemetry, dict)
    assert telemetry.get("additionalProperties") is False
    assert telemetry["properties"]["telemetry_schema"]["const"] == 2
    assert telemetry["properties"]["load_basis_points"]["maximum"] == 10_000
    assert {
        "process_instance_id",
        "sequence",
        "observed_at_unix_ms",
        "uptime_seconds",
        "active_sessions",
        "pending_pairs",
        "bandwidth_bps",
        "capacity_sessions",
        "draining",
        "admission_open",
        "probe_rate_limited",
        "telemetry_auth_failures",
    } <= set(telemetry["required"])
    fast_media = telemetry["properties"]["fast_media"]
    assert fast_media["additionalProperties"] is False
    assert {"active_allocations", "active_streams", "listener_failures"} <= set(
        fast_media["required"]
    )
    telemetry_example = read_json(
        CONTRACTS / "relay-telemetry/v2/telemetry.example.json"
    )
    assert isinstance(telemetry_example, dict)
    assert set(telemetry_example) == set(telemetry["required"])
    assert telemetry_example["telemetry_schema"] == 2
    assert telemetry_example["relay_probe_protocol"] == 1
    assert telemetry_example["relay_load_protocol"] == 1
    assert set(telemetry_example["fast_media"]) == set(fast_media["required"])
    assert telemetry_example["fast_media"]["protocol"] == 1


def check_openapi_surface() -> None:
    text = (CONTRACTS / "control/v1/openapi.yaml").read_text(encoding="utf-8")
    required = {
        "/capabilities",
        "/status",
        "/peers:verify",
        "/relays",
        "/relay-enrollments",
        "/relay-enrollments/{id}",
        "/relay-enrollments:prepare",
        "/relay-enrollments:complete",
        "/relay-enrollments:activate",
        "/relay-enrollments:revoke",
        "/allocations:simulate",
        "/config/schema",
        "/config",
        "/config:validate",
        "/config:plan",
        "/config:apply",
        "/config/history",
        "/config:rollback",
        "/operations/{id}",
        "/runtime:reload",
    }
    declared = {
        line.strip()[:-1]
        for line in text.splitlines()
        if line.startswith("  /") and line.endswith(":")
    }
    missing = required - declared
    assert not missing, f"OpenAPI is missing paths: {sorted(missing)}"
    assert "type: mutualTLS" in text
    assert "bearerFormat: JWT" in text
    assert "Idempotency-Key" in text and "If-Match" in text
    assert "SchemaBundle" in text
    assert "StrongETag" in text
    assert "required: [status, generation, subsystem_acks, etag, drift, document, format]" in text
    for forbidden in ("/commands", "/exec", "docker.sock", "21115"):
        assert forbidden not in text, f"forbidden remote surface in OpenAPI: {forbidden}"

    examples = CONTRACTS / "control/v1/examples"
    required_examples = {
        "allocation-simulation.json",
        "capabilities.json",
        "config.json",
        "history.json",
        "operation.json",
        "peer-verification.json",
        "plan.json",
        "relays.json",
        "status.json",
        "validation.json",
        "relay-enrollment-prepare.json",
        "relay-enrollment-complete.json",
        "relay-enrollment-activate.json",
        "relay-enrollments.json",
    }
    assert required_examples <= {path.name for path in examples.glob("*.json")}

    config_example = read_json(examples / "config.json")
    assert isinstance(config_example, dict)
    assert config_example.get("format") == "yaml"
    assert isinstance(config_example.get("document"), str)
    assert config_example["document"].startswith("version: 5\n")
    document_digest = "sha256:" + hashlib.sha256(
        config_example["document"].encode("utf-8")
    ).hexdigest()
    assert config_example["etag"] == json.dumps(document_digest)
    assert config_example["source_digest"] == document_digest

    capabilities = read_json(examples / "capabilities.json")
    assert isinstance(capabilities, dict)
    schema_bytes = (CONTRACTS / "config/v5/config.schema.json").read_bytes()
    assert capabilities["config"]["schema_digest"] == (
        "sha256:" + hashlib.sha256(schema_bytes).hexdigest()
    )
    assert capabilities["capabilities"]["config_schema"] == 5
    assert capabilities["config"]["supported_schema_versions"] == [1, 2, 3, 4, 5]
    assert capabilities["config"]["active_schema_version"] == 5
    assert capabilities["capabilities"]["peer_registry"] == 2
    assert capabilities["capabilities"]["profile_activation_lease"] == 1
    assert capabilities["capabilities"]["relay_probe_protocol"] == 1
    assert capabilities["capabilities"]["relay_load_protocol"] == 1
    assert capabilities["capabilities"]["relay_telemetry_schema"] == 2
    assert capabilities["capabilities"]["fast_media_relay_udp"] == 1
    assert capabilities["capabilities"]["starry_pairing"] == 1
    assert capabilities["capabilities"]["relay_enrollment"] == 1

    status = read_json(examples / "status.json")
    relays = read_json(examples / "relays.json")
    simulation = read_json(examples / "allocation-simulation.json")
    validation = read_json(examples / "validation.json")
    plan = read_json(examples / "plan.json")
    operation = read_json(examples / "operation.json")
    peer_verification = read_json(examples / "peer-verification.json")
    history = read_json(examples / "history.json")
    assert isinstance(status, dict) and {"ready", "config", "auth"} <= status.keys()
    assert isinstance(relays, dict) and isinstance(relays.get("relays"), list)
    assert isinstance(relays.get("fast_relay"), dict)
    assert isinstance(relays.get("profile_activation"), dict)
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    assert relays["relays"][0]["version"].endswith(f"-patch-v{patch_version}")
    assert relays["quality"]["protocol_version"] == 1
    assert relays["relays"][0]["capabilities"] == {
        "relay_probe_protocol": 1,
        "relay_load_protocol": 1,
        "fast_media_relay_udp": 1,
    }
    assert relays["relays"][0]["quality_candidate"] is True
    assert relays["relays"][0]["websocket"]["stale"] is False
    assert relays["relays"][0]["websocket"]["age_seconds"] >= 0
    assert relays["relays"][0]["websocket"]["url"].endswith("/ws/telemetry")
    assert relays["relays"][0]["websocket"]["telemetry_schema"] == 2
    assert relays["relays"][0]["websocket"]["pending_pairs"] >= 0
    assert relays["relays"][0]["websocket"]["bandwidth_ema_alpha_basis_points"] == 2_500
    assert {
        "offers_skipped",
        "offer_skip_reasons",
        "controller_reports_accepted",
        "reports_accepted",
        "reports_late",
        "reports_invalid",
        "reports_binding_mismatch",
        "fallback_reasons",
        "cache_hits",
        "hysteresis_decisions",
        "relay_selections",
    } <= relays["quality"].keys()
    serialized_relays = json.dumps(relays).lower()
    for forbidden in ("client_ip", "session_uuid", "allocation_id", "nonce", "raw_report"):
        assert forbidden not in serialized_relays
    assert relays["profile_activation"]["protocol_version"] == 1
    assert relays["profile_activation"]["burst_limit"] == 12
    assert relays["relays"][0]["fast_media_udp"]["active_streams"] >= 0
    assert relays["fast_relay"]["active_fast_media_authorizations"] >= 0
    assert relays["fast_relay"]["fast_media_v1_enabled"] is False
    assert relays["relays"][0]["websocket"]["load_basis_points"] <= 10000
    assert isinstance(simulation, dict) and simulation["selection"]["non_binding"] is True
    assert isinstance(validation, dict) and isinstance(validation.get("diagnostics"), list)
    assert isinstance(plan, dict) and plan["instance_id"] == capabilities["instance"]["id"]
    assert isinstance(operation, dict) and operation["id"]
    assert isinstance(peer_verification, dict)
    assert peer_verification["instance_id"] == capabilities["instance"]["id"]
    assert isinstance(peer_verification["registered"], bool)
    assert isinstance(history, dict) and isinstance(history.get("revisions"), list)
    enrollments = read_json(examples / "relay-enrollments.json")
    prepared = read_json(examples / "relay-enrollment-prepare.json")
    completed = read_json(examples / "relay-enrollment-complete.json")
    activation = read_json(examples / "relay-enrollment-activate.json")
    assert (
        enrollments["version"]
        == prepared["version"]
        == completed["version"]
        == activation["version"]
        == 1
    )
    assert prepared["state"] == "pending_claim"
    assert completed["state"] == "claimed_pending_health"
    assert enrollments["items"][0]["state"] == "active"
    assert enrollments["items"][0]["activation_operation_id"] == activation["operation_id"]
    assert "telemetry_secret" not in json.dumps(enrollments)
    assert completed["bundle"]["telemetry_secret"] == "A" * 43


def check_auth_fixtures() -> None:
    fixture_dir = CONTRACTS / "auth/v1/fixtures"
    expected = {
        "active.jwt.txt": (
            ["kessoku-api", "rustdesk-connect"],
            "606d24b7dc84f9392efb4f4c77a8e341ca48031a500a9353468a1c5a29ca64e3",
        ),
        "expired.jwt.txt": (
            ["kessoku-api", "rustdesk-connect"],
            "917b30df1f82594fec9cca30680c4e75b38c591685c6c4bc31bff1492e7f3cad",
        ),
        "wrong-audience.jwt.txt": (
            ["kessoku-api", "wrong-audience"],
            "79f225fd162f44359276492fb9e55f4ba9cb431ed10e7a92c8b1b83a5c410d33",
        ),
    }
    for name, (audience, expected_digest) in expected.items():
        raw = (fixture_dir / name).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == expected_digest, (
            f"{name} changed without an explicit contract-fixture review"
        )
        token = raw.decode("utf-8").strip()
        parts = token.split(".")
        assert len(parts) == 3, f"{name} is not a compact JWT"
        header = decode_jwt_part(parts[0])
        claims = decode_jwt_part(parts[1])
        assert header == {
            "alg": "EdDSA",
            "kid": "kessoku-fixture-2030-01",
            "typ": "at+jwt",
        }
        assert claims["aud"] == audience
        assert claims["iss"] == "https://api.example.test"
        assert claims["token_use"] == "access"
        assert "connect:initiate" in claims["scope"]
        assert isinstance(claims["user_id"], int) and claims["user_id"] > 0
        assert claims["sub"] == str(claims["user_id"])
        assert isinstance(claims["auth_version"], int) and claims["auth_version"] > 0


def check_patch_v131_frozen_candidate() -> None:
    summary_path = CONTRACTS / "patch-v1.3.1/CONTRACT-RELEASE-SUMMARY.json"
    candidate_files = sorted(
        path.relative_to(ROOT).as_posix()
        for path in summary_path.parent.iterdir()
        if path.is_file()
    )
    assert candidate_files == [
        "contracts/patch-v1.3.1/CONTRACT-RELEASE-SUMMARY.json"
    ], "patch-v1.3.1 must have exactly one canonical contract candidate summary"

    manifest = read_json(summary_path)
    assert isinstance(manifest, dict)
    assert manifest["manifest_schema"] == 1
    assert manifest["id"] == "patch-v1.3.1-contract-candidate"
    assert manifest["patch_version"] == "1.3.1"
    assert manifest["status"] == "FROZEN"
    assert manifest["candidate_kind"] == "CONTRACT_ONLY"
    assert manifest["runtime_release_status"] == "BLOCKED"
    assert manifest["hash_algorithm"] == "SHA-256"
    assert manifest["source_binding"] == {
        "type": "git_commit_containing_this_summary",
        "remote": "origin",
        "branch": "codex/patch-v1.3.1-fastmedia-pairing",
        "replacement_policy": "new_patch_version_required",
    }
    capability = manifest["schema_v5_capability"]
    assert capability["expression"] == "capabilities.config_schema"
    assert capability["json_pointer"] == "/capabilities/config_schema"
    assert capability["value"] == 5
    assert capability["supported_versions_path"] == (
        "config.supported_schema_versions"
    )
    assert capability["supported_versions"] == [1, 2, 3, 4, 5]
    assert capability["active_version_path"] == "config.active_schema_version"
    assert capability["schema_digest_path"] == "config.schema_digest"
    assert capability["version_string_inference_forbidden"] is True

    expected_files = {
        "control_openapi": "contracts/control/v1/openapi.yaml",
        "config_schema_v5": "contracts/config/v5/config.schema.json",
        "config_ui_schema_v5": "contracts/config/v5/config.ui-schema.json",
        "downgrade_drain_state_v1": (
            "contracts/config/v5/downgrade-drain-state.schema.json"
        ),
        "control_capabilities_fixture": (
            "contracts/control/v1/examples/capabilities.json"
        ),
        "control_relays_fixture": "contracts/control/v1/examples/relays.json",
        "control_status_fixture": "contracts/control/v1/examples/status.json",
        "relay_enrollment_prepare_fixture": (
            "contracts/control/v1/examples/relay-enrollment-prepare.json"
        ),
        "relay_enrollment_complete_fixture": (
            "contracts/control/v1/examples/relay-enrollment-complete.json"
        ),
        "relay_enrollment_activate_fixture": (
            "contracts/control/v1/examples/relay-enrollment-activate.json"
        ),
        "relay_enrollment_inventory_fixture": (
            "contracts/control/v1/examples/relay-enrollments.json"
        ),
        "fast_relay_authorization_v1": (
            "contracts/fast-relay/v1/rendezvous-extension.proto"
        ),
        "akr1_wire_v1": "contracts/fast-media/v1/akr1-wire.json",
        "sp1_pairing_v1": "contracts/starry-pairing/v1/pairing.schema.json",
        "relay_telemetry_schema_v2": (
            "contracts/relay-telemetry/v2/telemetry.schema.json"
        ),
        "relay_telemetry_fixture_v2": (
            "contracts/relay-telemetry/v2/telemetry.example.json"
        ),
    }
    frozen_files = manifest["files"]
    assert isinstance(frozen_files, list)
    assert len(frozen_files) == len(expected_files)
    assert {item["id"]: item["path"] for item in frozen_files} == expected_files
    assert len({item["id"] for item in frozen_files}) == len(frozen_files)
    assert len({item["path"] for item in frozen_files}) == len(frozen_files)
    for item in frozen_files:
        digest = "sha256:" + hashlib.sha256(
            (ROOT / item["path"]).read_bytes()
        ).hexdigest()
        assert item["sha256"] == digest, (
            f"frozen contract digest mismatch: {item['path']}"
        )

    inherited = manifest["inherited_frozen_contracts"]
    assert inherited == [
        {
            "id": "relay-quality/v1",
            "path": "contracts/relay-quality/v1/rendezvous-extension.proto",
            "sha256": (
                "sha256:19380d18aebf91a6856e43009291c79c"
                "f002d7c843b9fd7fa92c570916b2734c"
            ),
        }
    ]
    inherited_path = ROOT / inherited[0]["path"]
    assert inherited[0]["sha256"] == (
        "sha256:" + hashlib.sha256(inherited_path.read_bytes()).hexdigest()
    )

    expected_gates = {
        "AKARI_DUAL_ROLE_END_TO_END",
        "RELIABLE_FALLBACK_AUTO_REENTRY",
        "DEVICE_NETWORK_FAULT_MATRIX",
        "SP1_BROKER_CERT_ROTATION_MULTIHOST",
        "HOSTED_RELEASE_CI_ATTESTATION",
    }
    gates = manifest["runtime_release_gates"]
    assert {gate["code"] for gate in gates} == expected_gates
    assert all(gate["status"] == "BLOCKED" for gate in gates)
    assert len(gates) == len(expected_gates)
    assert manifest["kessoku_policy"] == {
        "minimum_version": "3.0.8",
        "pin": "exact_remote_commit_containing_this_summary",
        "dirty_worktree_forbidden": True,
        "runtime_release_approval": False,
    }


def check_release_version() -> None:
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    assert re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", patch_version)
    agent = (ROOT / "overlay/src/control_agent.rs").read_text(encoding="utf-8")
    assert (
        f'const STARRY_PATCH_VERSION: &str = "{patch_version}";' in agent
    ), "Control Agent capability version must match PATCH_VERSION"
    overlay = (ROOT / "scripts/apply_overlay.py").read_text(encoding="utf-8")
    assert 'repo_root / "PATCH_VERSION"' in overlay
    assert 'include_str!(\\"../PATCH_VERSION\\").trim()' in overlay
    assert '"x-starry-version"' in overlay
    for public_capability_header in (
        "x-starry-relay-probe-protocol",
        "x-starry-relay-load-protocol",
    ):
        assert public_capability_header in overlay
    for authenticated_telemetry_header in (
        "x-starry-telemetry-timestamp",
        "x-starry-telemetry-nonce",
        "x-starry-telemetry-auth",
        "x-starry-telemetry",
    ):
        assert authenticated_telemetry_header in overlay
    assert "STARRY_RELAY_TELEMETRY_SECRET_FILE" in overlay
    assert "response.load.is_none()" in (
        ROOT / "overlay/tests/mixed_relay.rs"
    ).read_text(encoding="utf-8")

    summary = build_summary(
        release_tag=f"1.1.16-patch-v{patch_version}",
        source_commit="1" * 40,
        upstream_ref="1.1.16",
        upstream_commit="2" * 40,
        upstream_hbb_common_commit="3" * 40,
        image_reference=(
            f"ghcr.io/q1ngyang/rustdesk-server-starry:"
            f"1.1.16-patch-v{patch_version}"
        ),
        image_index_digest="sha256:" + "4" * 64,
        image_linux_amd64_digest="sha256:" + "5" * 64,
    )
    contracts = summary["contracts"]
    assert isinstance(contracts, dict)
    assert contracts["control_openapi"]["digest"] == (
        "sha256:" + hashlib.sha256(
            (CONTRACTS / "control/v1/openapi.yaml").read_bytes()
        ).hexdigest()
    )
    assert contracts["config_schema"]["digest"] == (
        "sha256:" + hashlib.sha256(
            (CONTRACTS / "config/v5/config.schema.json").read_bytes()
        ).hexdigest()
    )
    assert contracts["config_ui_schema"]["digest"] == (
        "sha256:" + hashlib.sha256(
            (CONTRACTS / "config/v5/config.ui-schema.json").read_bytes()
        ).hexdigest()
    )
    candidate = contracts["contract_candidate"]
    assert isinstance(candidate, dict)
    manifest_path = CONTRACTS / "patch-v1.3.1/CONTRACT-RELEASE-SUMMARY.json"
    manifest = read_json(manifest_path)
    assert candidate["digest"] == (
        "sha256:" + hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    )
    assert candidate["path"] == (
        "contracts/patch-v1.3.1/CONTRACT-RELEASE-SUMMARY.json"
    )
    for key, value in manifest.items():
        assert candidate[key] == value


def check_relay_quality_protocol() -> None:
    contract = (
        CONTRACTS / "relay-quality/v1/rendezvous-extension.proto"
    ).read_text(encoding="utf-8")
    overlay = (ROOT / "scripts/apply_overlay.py").read_text(encoding="utf-8")
    implementation = (ROOT / "overlay/src/relay_quality.rs").read_text(encoding="utf-8")
    for message in (
        "StarryRelayLoad",
        "RelayProbeRequest",
        "RelayProbeResponse",
        "RelayQualityOffer",
        "RelayProbeReport",
        "RelayQualityDecision",
        "RelayQualityCancel",
    ):
        assert f"message {message} {{" in contract
    for capability_field in (
        "uint32 relay_probe_protocol = 5;",
        "uint32 relay_load_protocol = 6;",
        "uint32 probe_timeout_ms = 9;",
        "uint32 strategy = 10;",
        "uint32 stage = 11;",
        "bytes stage_token = 12;",
        "uint32 reason_code = 7;",
    ):
        assert capability_field in contract
        assert capability_field in overlay
    assert 'upstream / "contracts/relay-quality/v1/rendezvous-extension.proto"' in overlay
    assert 'marker = "message StarryRelayLoad {"' in overlay
    bindings = {
        "uint32 relay_quality_protocol = 100;",
        "RelayQualityOffer relay_quality_offer = 100;",
        "RelayProbeReport relay_quality_peer_report = 101;",
        "RelayQualityDecision relay_quality_decision = 102;",
        "RelayProbeRequest relay_probe_request = 100;",
        "RelayProbeResponse relay_probe_response = 101;",
        "RelayProbeReport relay_quality_stage_report = 102;",
        "RelayQualityOffer relay_quality_stage_offer = 103;",
        "RelayQualityDecision relay_quality_stage_decision = 104;",
        "RelayQualityCancel relay_quality_cancel = 105;",
    }
    for line in bindings:
        assert line in overlay
        tag = int(re.search(r"= (\d+);", line).group(1))
        assert tag >= 100, f"private extension tag is not high and additive: {line}"
    for scoring_control in (
        "effective_rtt",
        "jitter_penalty",
        "loss_penalty",
        "load_penalty",
        "hysteresis_basis_points",
        "missing_report_penalty_basis_points",
        "primary_is_good_enough",
        "transition_to_expanded",
        "finalize_expired",
    ):
        assert scoring_control in implementation
    assert "reason: String::new()" in implementation
    assert "load:" not in implementation[implementation.index("fn wire_candidate"):implementation.index("fn offer_for")]
    frozen = (CONTRACTS / "relay-quality/v1/FROZEN").read_text(encoding="utf-8")
    digest = hashlib.sha256(contract.encode("utf-8")).hexdigest()
    assert "status: FROZEN" in frozen
    assert f"canonical_sha256: {digest}" in frozen
    assert "official_server_commit: 73523b31cfd25d77dee862e6fc9f5e1fb5e485ef" in frozen


def check_fast_relay_protocol() -> None:
    contract = (
        CONTRACTS / "fast-relay/v1/rendezvous-extension.proto"
    ).read_text(encoding="utf-8")
    overlay = (ROOT / "scripts/apply_overlay.py").read_text(encoding="utf-8")
    implementation = (ROOT / "overlay/src/fast_relay.rs").read_text(encoding="utf-8")
    assert "message FastRelayAuthorization {" in contract
    for field in (
        "uint32 version = 1;",
        "string session_uuid = 2;",
        "uint64 expires_at = 3;",
        "bool allow_fast_compat = 4;",
        "bool allow_fast_media_v1 = 5;",
        "uint32 max_bitrate_kbps = 6;",
        "uint32 relay_udp_protocol = 7;",
        "string relay_server = 8;",
        "uint32 relay_udp_port = 9;",
        "bytes relay_allocation_id = 10;",
        "uint32 relay_max_datagram = 11;",
        "uint32 relay_endpoint_role = 12;",
    ):
        assert field in contract
    assert 'upstream / "contracts/fast-relay/v1/rendezvous-extension.proto"' in overlay
    assert '"bytes fast_relay_authorization = 64;"' in overlay
    for control in (
        "ENDPOINT_CONTROLLER",
        "ENDPOINT_TARGET",
        "relay_allocation_id",
        "selected_relay",
        "reliable_fallbacks",
    ):
        assert control in implementation
    assert "sign::sign(&payload, signing_key)" in implementation
    assert "fast_relay::authorization_for_request" in overlay
    assert "fast_relay::authorization_for_response" in overlay


def check_fast_media_and_pairing_protocols() -> None:
    wire = read_json(CONTRACTS / "fast-media/v1/akr1-wire.json")
    assert isinstance(wire, dict)
    assert wire["version"] == 1
    assert wire["status"] == "FROZEN"
    assert wire["runtime_release_status"] == "BLOCKED"
    assert wire["header"]["bytes"] == 32
    assert wire["authorization"]["required_fields"] == list(range(1, 13))
    assert wire["authorization"]["relay_max_datagram_range"] == [608, 1400]
    assert wire["privacy"]["relay_has_media_keys"] is False
    implementation = (ROOT / "overlay/src/fast_media_relay.rs").read_text(
        encoding="utf-8"
    )
    for invariant in (
        'b"AKR1"',
        'b"AKF1"',
        "MAX_AUTHORIZATION_BYTES: usize = 4_096",
        "MIN_RELAY_DATAGRAM: u32 = 608",
        "MAX_RELAY_DATAGRAM: u32 = 1_400",
        "saturating_mul(145)",
        "ReplayWindow",
        "allow_rebind",
    ):
        assert invariant in implementation

    pairing = read_json(CONTRACTS / "starry-pairing/v1/pairing.schema.json")
    assert isinstance(pairing, dict)
    definitions = pairing["$defs"]
    assert definitions["pairingCode"]["pattern"].startswith("^SP1")
    assert definitions["pairingCodePayload"]["additionalProperties"] is False
    assert definitions["claimRequest"]["additionalProperties"] is False
    assert definitions["relayBundle"]["properties"]["telemetry_secret"][
        "x-sensitive"
    ] is True
    pairing_source = (ROOT / "overlay/src/pairing.rs").read_text(encoding="utf-8")
    for binding in (
        "broker_spki_sha256",
        "configuration_digest",
        "request_digest",
        "key_fingerprint",
        "csr_digest",
        "pairing_pending_identity_changed",
    ):
        assert binding in pairing_source


def check_profile_activation_protocol() -> None:
    contract = (
        CONTRACTS / "profile-activation/v1/rendezvous-extension.proto"
    ).read_text(encoding="utf-8")
    overlay = (ROOT / "scripts/apply_overlay.py").read_text(encoding="utf-8")
    implementation = (ROOT / "overlay/src/profile_activation.rs").read_text(
        encoding="utf-8"
    )
    assert "message DeactivatePeer {" in contract
    assert "message DeactivatePeerResponse {" in contract
    for field in (
        "bytes network_identity_uuid = 2;",
        "bytes activation_id = 4;",
        "bytes route_lease = 5;",
        "uint64 route_generation = 6;",
    ):
        assert field in contract
    for additive in (
        '"uint64 route_generation = 60;"',
        '"uint64 activation_epoch = 61;"',
        '"bytes route_lease = 62;"',
        '"bytes activation_id = 63;"',
        '"DeactivatePeer deactivate_peer = 62;"',
        '"DeactivatePeerResponse deactivate_peer_response = 63;"',
    ):
        assert additive in overlay
    assert "BURST_LIMIT: usize = 12" in implementation
    assert "BURST_WINDOW_SECONDS: u64 = 30" in implementation
    assert "LEASE_TTL_SECONDS: u64 = 45" in implementation
    assert "public_key_sha256" in implementation
    assert "disconnect_route" in implementation


def main() -> None:
    check_json_contracts()
    check_openapi_surface()
    check_auth_fixtures()
    check_patch_v131_frozen_candidate()
    check_release_version()
    check_relay_quality_protocol()
    check_fast_relay_protocol()
    check_fast_media_and_pairing_protocols()
    check_profile_activation_protocol()
    print("Starry contracts are structurally valid and least-privilege surface checks passed")


if __name__ == "__main__":
    main()
