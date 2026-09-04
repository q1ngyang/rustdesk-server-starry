#!/usr/bin/env python3
"""Dependency-free Relay Reallocation v1 freeze/fixture consistency checks."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "contracts/relay-reallocation/v1"
FIX = BASE / "fixtures"
SCHEMA = ROOT / "contracts/config/v6/config.schema.json"
UI = ROOT / "contracts/config/v6/config.ui-schema.json"
OPENAPI = ROOT / "contracts/control/v2/openapi.yaml"
SUMMARY = BASE / "CONTRACT-RELEASE-SUMMARY.json"


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def walk_keys(value):
    if isinstance(value, dict):
        for key, child in value.items():
            yield key
            yield from walk_keys(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_keys(child)


def main() -> None:
    contract = load(BASE / "contract.json")
    schema = load(SCHEMA)
    ui = load(UI)
    capabilities = load(FIX / "capabilities.json")
    config = load(FIX / "config-document.json")
    status = load(FIX / "relays-status.json")
    missing = load(FIX / "capability-missing.json")
    unknown = load(FIX / "capability-unknown-version.json")
    legacy_control = load(FIX / "legacy-control-v1.json")
    legacy_config = load(FIX / "legacy-starry-config-v5.json")

    assert contract["status"] == "FROZEN"
    assert contract["contract_version"] == 1
    assert contract["control_api"] == {"version": "1.2.0", "openapi_contract_version": 2}
    assert contract["config_schema_version"] == 6
    assert contract["capability"]["name"] == "relay_reallocation"
    assert contract["capability"]["version"] == 1

    proto = (BASE / "rendezvous-extension.proto").read_text(encoding="utf-8")
    arms = contract["rendezvous_oneof_tags"]
    assert sorted(arms.values()) == list(range(108, 117))
    for name, tag in arms.items():
        assert re.search(rf"\b{name.replace('_', '.?')}\b", name) is not None  # stable keys
        assert f"= {tag};" in proto
    assert "relay_reallocation_protocol = 103;" in proto
    assert "relay_reallocation_protocol = 102;" in proto
    assert len(set(arms.values())) == len(arms)

    assert schema["properties"]["version"]["enum"] == [1, 2, 3, 4, 5, 6]
    rr = schema["$defs"]["relayReallocation"]
    assert rr["additionalProperties"] is False
    props = rr["properties"]
    expected_defaults = {
        "enabled": False, "policy": "auto",
        "allowed_initiator_roles": ["controller", "target"],
        "required_scope": "starry.relay.reallocate", "require_peer_confirmation": True,
        "probe_timeout_ms": 5000, "prepare_timeout_ms": 8000,
        "commit_timeout_ms": 10000, "rollback_timeout_ms": 5000,
        "cooldown_seconds": 30, "max_candidates": 3, "max_active": 10000,
        "per_session_per_minute": 2, "per_ip_per_minute": 20,
        "global_per_minute": 1000,
    }
    for key, value in expected_defaults.items():
        assert props[key]["default"] == value, key
    assert props["policy"]["enum"] == contract["configuration"]["policies"]
    assert ui["relay_reallocation"]["required_scope"]["ui:readonly"] is True
    relay_endpoint = schema["$defs"]["relayEndpoint"]
    assert relay_endpoint["additionalProperties"] is False
    for key in ("node_id", "display_name", "region"):
        assert key in relay_endpoint["properties"]

    openapi = OPENAPI.read_text(encoding="utf-8")
    for needle in (
        "version: 1.2.0", "relay_reallocation: {type: integer, const: 1}",
        "config_schema: {type: integer, const: 6}", "additionalProperties: false",
        "/relay-reallocation/status:", "runtime_release_status: {const: IMPLEMENTATION_PENDING}",
        "enum: [auto, fixed, force-auto, force-fixed]",
    ):
        assert needle in openapi, needle
    forbidden_openapi_properties = (
        "session_uuid", "request_id", "reallocation_id", "allocation_id",
        "client_address", "ip", "probe_url", "raw_report", "nonce", "token",
        "grant", "key", "capacity", "bandwidth",
    )
    for key in forbidden_openapi_properties:
        assert re.search(rf"(?m)^\s+{re.escape(key)}\s*:", openapi) is None, key

    assert capabilities["protocol"]["version"] == "1.2.0"
    assert capabilities["capabilities"]["relay_reallocation"] == 1
    assert capabilities["capabilities"]["config_schema"] == 6
    assert capabilities["config"]["schema_digest"] == digest(SCHEMA)
    assert config["version"] == 6
    assert config["relay_reallocation"]["policy"] in props["policy"]["enum"]
    assert status["contract_version"] == status["capability_version"] == 1
    assert status["config_schema_version"] == 6
    assert status["runtime_release_status"] == "IMPLEMENTATION_PENDING"
    assert set(status["aggregates"]) == set(contract["aggregate_dimensions"])
    assert status["policy"] in contract["configuration"]["policies"]
    forbidden_fixture = set(forbidden_openapi_properties)
    assert not (set(walk_keys(status)) & forbidden_fixture)
    assert missing["expected"]["relay_reallocation_supported"] is False
    assert unknown["input"]["capabilities"]["relay_reallocation"] != 1
    assert unknown["expected"]["action"] == "fail_closed_keep_current_reliable_session"
    assert legacy_control["control_api_version"] == "1.1.0"
    assert "relay_reallocation" not in legacy_control["capabilities"]
    assert legacy_config["version"] == 5

    if SUMMARY.exists():
        summary = load(SUMMARY)
        assert summary["contract_status"] == "FROZEN"
        assert summary["runtime_release_status"] == "IMPLEMENTATION_PENDING"
        for item in summary["files"]:
            assert digest(ROOT / item["path"]) == item["sha256"], item["path"]
        inherited = {item["name"]: item["sha256"] for item in summary["inherited_contracts"]}
        assert inherited["relay_quality"] == digest(ROOT / "contracts/relay-quality/v1/rendezvous-extension.proto")
        assert inherited["fast_relay_authorization"] == digest(ROOT / "contracts/fast-relay/v1/rendezvous-extension.proto")
        assert inherited["fast_media_relay_udp"] == digest(ROOT / "contracts/fast-media/v1/akr1-wire.json")
        assert inherited["fast_media_relay_renewal"] == digest(ROOT / "contracts/fast-media-renewal/v1/rendezvous-extension.proto")
        assert inherited["relay_telemetry_schema"] == digest(ROOT / "contracts/relay-telemetry/v3/telemetry.schema.json")
        claimed = summary["summary_sha256"]
        summary["summary_sha256"] = ""
        canonical = json.dumps(summary, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
        assert hashlib.sha256(canonical).hexdigest() == claimed

    print("Relay Reallocation v1 frozen contract, schemas, OpenAPI and fixtures are consistent")


if __name__ == "__main__":
    main()
