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

    return {
        "schema_version": 1,
        "release": {
            "tag": release_tag,
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
            "control_openapi": {
                "id": "control/v1",
                "path": "contracts/control/v1/openapi.yaml",
                "digest": sha256("contracts/control/v1/openapi.yaml"),
            },
            "config_schema": {
                "id": "config/v4",
                "path": "contracts/config/v4/config.schema.json",
                "digest": sha256("contracts/config/v4/config.schema.json"),
            },
            "config_ui_schema": {
                "id": "config/v4-ui",
                "path": "contracts/config/v4/config.ui-schema.json",
                "digest": sha256("contracts/config/v4/config.ui-schema.json"),
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
                "id": "relay-telemetry/v1",
                "path": "contracts/relay-telemetry/v1/telemetry.schema.json",
                "digest": sha256(
                    "contracts/relay-telemetry/v1/telemetry.schema.json"
                ),
            },
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
        )
        with args.output.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(summary, stream, indent=2, sort_keys=True)
            stream.write("\n")
    except (OSError, ValueError) as error:
        parser.exit(1, f"release summary error: {error}\n")


if __name__ == "__main__":
    main()
