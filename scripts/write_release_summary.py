#!/usr/bin/env python3
"""Write the privacy-safe, immutable Starry release/contract digest summary."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
RELEASE_TAG = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[._-][A-Za-z0-9.-]+)?-patch-v[0-9]+\.[0-9]+\.[0-9]+$")


def sha256(relative: str) -> str:
    return "sha256:" + hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()


def patch_tuple(value: str) -> tuple[int, int, int]:
    parts = value.split(".")
    if len(parts) != 3 or not all(part.isdigit() for part in parts):
        raise ValueError("patch version must contain three numeric components")
    return tuple(int(part) for part in parts)  # type: ignore[return-value]


def contract_candidate(patch_version: str) -> dict[str, object]:
    path = f"contracts/patch-v{patch_version}/CONTRACT-RELEASE-SUMMARY.json"
    if not (ROOT / path).is_file():
        raise ValueError(f"frozen contract candidate is missing for patch-v{patch_version}")
    manifest = json.loads((ROOT / path).read_text(encoding="utf-8"))
    return {
        "path": path,
        "digest": sha256(path),
        **manifest,
    }


def build_summary(
    *,
    release_tag: str,
    source_commit: str,
    upstream_ref: str,
    upstream_commit: str,
    upstream_hbb_common_commit: str,
    image_reference: str,
    image_index_digest: str,
    image_linux_amd64_digest: str,
    release_channel: str,
) -> dict[str, object]:
    if not RELEASE_TAG.fullmatch(release_tag):
        raise ValueError("release tag is not a Starry patch tag")
    for label, value in (
        ("source commit", source_commit),
        ("upstream commit", upstream_commit),
        ("upstream hbb_common commit", upstream_hbb_common_commit),
    ):
        if not HEX_40.fullmatch(value):
            raise ValueError(f"{label} must be a lowercase full commit")
    for label, value in (
        ("image index digest", image_index_digest),
        ("linux/amd64 image digest", image_linux_amd64_digest),
    ):
        if not DIGEST.fullmatch(value):
            raise ValueError(f"{label} must be sha256:<64 lowercase hex>")
    if image_reference != f"ghcr.io/q1ngyang/rustdesk-server-starry:{release_tag}":
        raise ValueError("image reference must use the immutable Starry release tag")
    if not upstream_ref or any(character.isspace() for character in upstream_ref):
        raise ValueError("upstream ref must be a non-empty token")
    if release_channel not in {"preview", "stable"}:
        raise ValueError("release channel must be preview or stable")
    patch_version = release_tag.rsplit("-patch-v", 1)[1]
    patch_release = patch_tuple(patch_version)
    telemetry_version = 3 if patch_release >= (1, 3, 2) else 2

    return {
        "schema_version": 1,
        "release": {
            "tag": release_tag,
            "channel": release_channel,
            "source_commit": source_commit,
            "upstream_repository": "rustdesk/rustdesk-server",
            "upstream_ref": upstream_ref,
            "upstream_commit": upstream_commit,
            "upstream_hbb_common_commit": upstream_hbb_common_commit,
        },
        "image": {
            "reference": image_reference,
            "index_digest": image_index_digest,
            "platforms": {"linux/amd64": image_linux_amd64_digest},
        },
        "contracts": {
            "contract_candidate": contract_candidate(patch_version),
            "control_openapi": {
                "id": "control/v1",
                "path": "contracts/control/v1/openapi.yaml",
                "digest": sha256("contracts/control/v1/openapi.yaml"),
            },
            "config_schema": {
                "id": "config/v5",
                "path": "contracts/config/v5/config.schema.json",
                "digest": sha256("contracts/config/v5/config.schema.json"),
            },
            "config_ui_schema": {
                "id": "config/v5-ui",
                "path": "contracts/config/v5/config.ui-schema.json",
                "digest": sha256("contracts/config/v5/config.ui-schema.json"),
            },
            "relay_quality_protocol": {
                "id": "relay-quality/v1",
                "status": "FROZEN",
                "path": "contracts/relay-quality/v1/rendezvous-extension.proto",
                "digest": sha256(
                    "contracts/relay-quality/v1/rendezvous-extension.proto"
                ),
            },
            "relay_telemetry_schema": {
                "id": f"relay-telemetry/v{telemetry_version}",
                "path": (
                    f"contracts/relay-telemetry/v{telemetry_version}/telemetry.schema.json"
                ),
                "digest": sha256(
                    f"contracts/relay-telemetry/v{telemetry_version}/telemetry.schema.json"
                ),
            },
            "fast_relay_protocol": {
                "id": "fast-relay/v1",
                "path": "contracts/fast-relay/v1/rendezvous-extension.proto",
                "digest": sha256(
                    "contracts/fast-relay/v1/rendezvous-extension.proto"
                ),
            },
            "fast_media_relay_udp": {
                "id": "fast-media/v1",
                "status": "FROZEN",
                "runtime_release_status": release_channel.upper(),
                "path": "contracts/fast-media/v1/akr1-wire.json",
                "digest": sha256("contracts/fast-media/v1/akr1-wire.json"),
            },
            "starry_pairing_protocol": {
                "id": "starry-pairing/v1",
                "path": "contracts/starry-pairing/v1/pairing.schema.json",
                "digest": sha256(
                    "contracts/starry-pairing/v1/pairing.schema.json"
                ),
            },
            "downgrade_drain_state": {
                "id": "config/v5-downgrade-drain-state/v1",
                "path": "contracts/config/v5/downgrade-drain-state.schema.json",
                "digest": sha256(
                    "contracts/config/v5/downgrade-drain-state.schema.json"
                ),
            },
            **(
                {
                    "fast_media_renewal_protocol": {
                        "id": "fast-media-renewal/v1",
                        "status": "FROZEN",
                        "runtime_release_status": release_channel.upper(),
                        "path": (
                            "contracts/fast-media-renewal/v1/"
                            "rendezvous-extension.proto"
                        ),
                        "digest": sha256(
                            "contracts/fast-media-renewal/v1/"
                            "rendezvous-extension.proto"
                        ),
                        "capability": {"fast_media_relay_renewal": 1},
                    }
                }
                if patch_release >= (1, 3, 2)
                else {}
            ),
        },
        "runtime_gates": {
            "preview": (
                "source, controlled-clock, contract, overlay, lint, test, and build gates"
            ),
            "stable_latest": [
                "real Akari controller/target long-session evidence",
                "cross-network NAT/UDP migration and fault soak",
                "hosted immutable artifact provenance",
            ],
        },
        "kessoku": {
            "minimum_version_for_renewal_aggregates": (
                "3.0.9" if patch_release >= (1, 3, 2) else None
            ),
            "media_or_signing_path": False,
            "process_instance_id_policy": (
                "discard_at_ingress_never_forward_persist_index_log_or_display"
            ),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--upstream-ref", required=True)
    parser.add_argument("--upstream-commit", required=True)
    parser.add_argument("--upstream-hbb-common-commit", required=True)
    parser.add_argument("--image-reference", required=True)
    parser.add_argument("--image-index-digest", required=True)
    parser.add_argument("--image-linux-amd64-digest", required=True)
    parser.add_argument("--release-channel", required=True)
    args = parser.parse_args()
    try:
        summary = build_summary(
            release_tag=args.release_tag,
            source_commit=args.source_commit,
            upstream_ref=args.upstream_ref,
            upstream_commit=args.upstream_commit,
            upstream_hbb_common_commit=args.upstream_hbb_common_commit,
            image_reference=args.image_reference,
            image_index_digest=args.image_index_digest,
            image_linux_amd64_digest=args.image_linux_amd64_digest,
            release_channel=args.release_channel,
        )
        with args.output.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(summary, stream, indent=2, sort_keys=True)
            stream.write("\n")
    except (OSError, ValueError) as error:
        parser.exit(1, f"release summary error: {error}\n")


if __name__ == "__main__":
    main()
