#!/usr/bin/env python3
"""Fail closed on malformed or accidentally broadened public contracts."""

from __future__ import annotations

import base64
import hashlib
import json
import re
from pathlib import Path


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

    schema = read_json(CONTRACTS / "config/v3/config.schema.json")
    assert isinstance(schema, dict)
    assert schema.get("additionalProperties") is False
    assert schema["properties"]["version"]["enum"] == [1, 2, 3]
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


def check_openapi_surface() -> None:
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    text = (CONTRACTS / "control/v1/openapi.yaml").read_text(encoding="utf-8")
    required = {
        "/capabilities",
        "/status",
        "/relays",
        "/peers:verify",
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
    }
    assert required_examples <= {path.name for path in examples.glob("*.json")}

    config_example = read_json(examples / "config.json")
    assert isinstance(config_example, dict)
    assert config_example.get("format") == "yaml"
    assert isinstance(config_example.get("document"), str)
    assert config_example["document"].startswith("version: 3\n")
    document_digest = "sha256:" + hashlib.sha256(
        config_example["document"].encode("utf-8")
    ).hexdigest()
    assert config_example["etag"] == json.dumps(document_digest)
    assert config_example["source_digest"] == document_digest

    capabilities = read_json(examples / "capabilities.json")
    assert isinstance(capabilities, dict)
    schema_bytes = (CONTRACTS / "config/v3/config.schema.json").read_bytes()
    assert capabilities["config"]["schema_digest"] == (
        "sha256:" + hashlib.sha256(schema_bytes).hexdigest()
    )
    assert capabilities["capabilities"]["peer_registry"] == 1

    peer_verification = read_json(examples / "peer-verification.json")
    assert peer_verification == {
        "instance_id": capabilities["instance"]["id"],
        "registered": True,
    }

    status = read_json(examples / "status.json")
    relays = read_json(examples / "relays.json")
    simulation = read_json(examples / "allocation-simulation.json")
    validation = read_json(examples / "validation.json")
    plan = read_json(examples / "plan.json")
    operation = read_json(examples / "operation.json")
    history = read_json(examples / "history.json")
    assert isinstance(status, dict) and {"ready", "config", "auth"} <= status.keys()
    assert isinstance(relays, dict) and isinstance(relays.get("relays"), list)
    assert relays["relays"][0]["version"].endswith(f"-patch-v{patch_version}")
    assert isinstance(simulation, dict) and simulation["selection"]["non_binding"] is True
    assert isinstance(validation, dict) and isinstance(validation.get("diagnostics"), list)
    assert isinstance(plan, dict) and plan["instance_id"] == capabilities["instance"]["id"]
    assert isinstance(operation, dict) and operation["id"]
    assert isinstance(history, dict) and isinstance(history.get("revisions"), list)


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


def main() -> None:
    check_json_contracts()
    check_openapi_surface()
    check_auth_fixtures()
    check_release_version()
    print("Starry contracts are structurally valid and least-privilege surface checks passed")


if __name__ == "__main__":
    main()
