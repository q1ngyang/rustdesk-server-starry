#!/usr/bin/env python3
"""Reject moving GitHub Action and Rust toolchain inputs."""

from __future__ import annotations

import hashlib
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
FULL_COMMIT = re.compile(r"^[0-9a-f]{40}$")
TOOLCHAIN = re.compile(r"^  RUST_TOOLCHAIN: ([0-9]+\.[0-9]+\.[0-9]+)$", re.MULTILINE)
CROSS_REVISION = re.compile(r"^  CROSS_REVISION: ([0-9a-f]{40})$", re.MULTILINE)
USES = re.compile(r"\buses:\s*[^\s@]+@([^\s#]+)")
PINNED_CROSS_IMAGE = re.compile(
    r'^image = "ghcr\.io/cross-rs/[a-z0-9_-]+@sha256:[0-9a-f]{64}"$',
    re.MULTILINE,
)


def main() -> None:
    paths = sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))
    assert paths, "no GitHub Actions workflows found"
    build = (WORKFLOWS / "build.yml").read_text(encoding="utf-8")
    match = TOOLCHAIN.search(build)
    assert match, "build workflow must pin RUST_TOOLCHAIN to X.Y.Z"
    toolchain_version = match.group(1)
    assert "CARGO_AUDIT_VERSION: 0.22.2" in build, (
        "cargo-audit must be fixed to the reviewed release"
    )
    assert (
        "RUSTSEC_ADVISORY_DB_REVISION: "
        "2f08fbb85332687b721f2f22706d07448369451b" in build
    ), "RustSec advisory database must be fixed to the reviewed commit"
    assert CROSS_REVISION.search(build), "build workflow must pin cross to a full commit"
    assert "houseabsolute/actions-rust-cross" not in build
    assert (
        "DEBIAN_TEST_IMAGE: debian:bookworm-slim@sha256:"
        "abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241"
        in build
    ), "Debian package installation image must match the reviewed manifest digest"
    assert build.count(
        "image: tonistiigi/binfmt@sha256:"
        "400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0"
    ) == 0, "the amd64 release workflow must not require QEMU/binfmt"
    assert build.count(
        "driver-opts: image=moby/buildkit@sha256:"
        "28a898719c18a33f4e8000685287fa36fd0dd9560c6440227d3a732d79bb41d8"
    ) == 2, "every Buildx setup must use the reviewed BuildKit manifest"
    for amd64_release_control in (
        "target: x86_64-unknown-linux-musl",
        "platforms: linux/amd64",
        "name: deb-amd64",
        "test \"$(find candidate/release-assets -maxdepth 1 -name '*.deb' | wc -l)\" -eq 4",
    ):
        assert amd64_release_control in build, (
            f"amd64 release scope is missing: {amd64_release_control}"
        )
    for unsupported_release_input in (
        "target: aarch64-unknown-linux-musl",
        "platforms: linux/amd64,linux/arm64",
        "name: linux-arm64",
        "name: deb-arm64",
    ):
        assert unsupported_release_input not in build, (
            f"non-blocking ARM input entered the release workflow: "
            f"{unsupported_release_input}"
        )
    assert "name: Experimental Windows amd64 compatibility" in build
    assert "continue-on-error: true" in build
    for source_control in (
        '"refs/tags/${UPSTREAM_REF}:refs/tags/${UPSTREAM_REF}"',
        'git -C _upstream rev-parse "${UPSTREAM_REF}^{commit}"',
        "upstream_commit: ${{ steps.upstream.outputs.upstream_commit }}",
        "upstream_common_commit: ${{ steps.upstream.outputs.upstream_common_commit }}",
        "upstream_commit=${UPSTREAM_COMMIT}",
        "upstream_hbb_common_commit=${UPSTREAM_COMMON_COMMIT}",
    ):
        assert source_control in build, (
            f"upstream source provenance is missing: {source_control}"
        )
    for dependency_lock_control in (
        "cmp overlay/Cargo.lock _upstream/Cargo.lock",
        "--format-version 1 --locked",
    ):
        assert dependency_lock_control in build, (
            f"fixed dependency verification is missing: {dependency_lock_control}"
        )
    assert build.count("cargo metadata --manifest-path _upstream/Cargo.toml") == 1, (
        "workflow must never run unlocked metadata before verifying Cargo.lock"
    )
    cross_config = (ROOT / "Cross.toml").read_text(encoding="utf-8")
    cross_images = set(PINNED_CROSS_IMAGE.findall(cross_config))
    assert cross_images == {
        'image = "ghcr.io/cross-rs/x86_64-unknown-linux-musl@sha256:d54fdde7f1b680901a0bb21a2952e4921172b94c17e48603ccbbaeca8b5ef7e8"',
        'image = "ghcr.io/cross-rs/aarch64-unknown-linux-musl@sha256:f604e399cbb2154ddeb013db99eb4f123d24f09a579c7e8d6ed631d15ffa8b12"',
    }, "both musl cross images must match the reviewed immutable digests"
    assert ":main" not in cross_config and ":latest" not in cross_config
    deb_builder = (ROOT / "scripts/build_deb.sh").read_text(encoding="utf-8")
    for reproducible_control in (
        "dpkg --validate-version",
        "[ -L \"$binary\" ]",
        "find \"$package_root\" -exec touch -h -d '@0' {} +",
        "SOURCE_DATE_EPOCH=0 dpkg-deb --build --root-owner-group -Zgzip",
    ):
        assert reproducible_control in deb_builder, (
            f"Debian package builder is missing reproducibility control: "
            f"{reproducible_control}"
        )
    dockerfile = (ROOT / "docker/Dockerfile").read_text(encoding="utf-8")
    assert (
        "FROM alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce"
        in dockerfile
    ), "runtime base image must match the reviewed multi-architecture digest"
    assert "apk add --no-cache ca-certificates=20260611-r0" in dockerfile, (
        "runtime CA package version must be fixed"
    )
    installer = (ROOT / "scripts/install_ci_tools.sh").read_text(encoding="utf-8")
    for pinned_input in (
        "actionlint_version=1.7.12",
        "actionlint_sha256=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8",
        "gitleaks_version=8.25.1",
        "gitleaks_sha256=3000d057342489827ee127310771873000b658f2987be7bbd21968ab7443913a",
        "syft_version=1.50.0",
        "syft_sha256=bf7b29ff57f06da30918266a0e1c2885a8f99784798d1bdb1628886aa015d788",
    ):
        assert pinned_input in installer, f"missing pinned CI tool input: {pinned_input}"
    assert "scripts/install_ci_tools.sh" in build
    assert 'gitleaks" git . --config .gitleaks.toml' in build
    for rust_audit_control in (
        "repository: RustSec/advisory-db",
        "ref: ${{ env.RUSTSEC_ADVISORY_DB_REVISION }}",
        'cargo install cargo-audit --version "${CARGO_AUDIT_VERSION}" --locked',
        'test "$(cargo-audit --version)" = "cargo-audit ${CARGO_AUDIT_VERSION}"',
        "cargo audit --db _rustsec-advisory-db --no-fetch",
        "--file _upstream/Cargo.lock --deny unsound --json",
        "starry-rustsec-audit-${{ github.sha }}",
    ):
        assert rust_audit_control in build, (
            f"Rust dependency audit is missing fixed control: {rust_audit_control}"
        )
    overlay = (ROOT / "scripts/apply_overlay.py").read_text(encoding="utf-8")
    assert 'repo_root / "overlay/Cargo.lock"' in overlay, (
        "the reviewed Cargo lockfile must be copied into patched source"
    )
    for quality_source_control in (
        "_upstream/libs/hbb_common/protos _upstream/PATCH_VERSION",
        "_upstream/src/relay_quality.rs",
        "_upstream/src/fast_relay.rs",
        "_upstream/src/profile_activation.rs",
        "--test mixed_relay",
        "--test protocol_contract",
    ):
        assert quality_source_control in build, (
            f"build workflow is missing Relay quality verification: {quality_source_control}"
        )
    lockfile = ROOT / "overlay/Cargo.lock"
    assert lockfile.is_file(), "the reviewed Cargo lockfile is missing"
    assert hashlib.sha256(lockfile.read_bytes()).hexdigest() == (
        "631d4772927be1d1e3568a3e3185d51e9a925ab49afae19cd520e12679783c36"
    ), "the fixed Cargo dependency graph changed without review"
    for dependency_control in (
        'tokio-rusqlite = { version = "0.7", features = ["bundled"] }',
        'jsonwebtoken = { version = "9.3.1", default-features = false }',
        'reqwest = { version = "0.12.28"',
    ):
        assert dependency_control in overlay, (
            f"overlay is missing reviewed dependency control: {dependency_control}"
        )
    lock_text = lockfile.read_text(encoding="utf-8")
    database_source = (ROOT / "overlay/src/database.rs").read_text(encoding="utf-8")
    for removed_dependency in ('name = "sqlx"', 'name = "deadpool"'):
        assert removed_dependency not in lock_text, (
            f"fixed graph still contains obsolete dependency: {removed_dependency}"
        )
    assert "sqlx" not in database_source and "deadpool" not in database_source, (
        "database overlay must use the reviewed tokio-rusqlite implementation"
    )
    for test_control in (
        "_upstream/src/database.rs",
        "ulimit -n 8192",
        "hbbs_sustains_one_thousand_registered_idle_websockets",
    ):
        assert test_control in build, f"build workflow is missing test control: {test_control}"
    websocket_gate = (ROOT / "overlay/tests/websocket_signal.rs").read_text(
        encoding="utf-8"
    )
    for websocket_load_control in (
        "ensure_websocket_load_nofile_limit();",
        "libc::getrlimit(libc::RLIMIT_NOFILE",
        "libc::setrlimit(libc::RLIMIT_NOFILE",
        "const REQUIRED: libc::rlim_t = 8_192;",
    ):
        assert websocket_load_control in websocket_gate, (
            "the 1,000-WebSocket gate must establish its own bounded file-descriptor "
            f"precondition: {websocket_load_control}"
        )
    release_gate = {}
    for line in (ROOT / "RELEASE_STATUS").read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition(":")
        if separator and not line.lstrip().startswith("#"):
            release_gate[key.strip()] = value.strip()
    assert release_gate.get("status") in {"BLOCKED", "APPROVED"}, (
        "RELEASE_STATUS status must be BLOCKED or APPROVED"
    )
    assert release_gate.get("patch_version") == (
        ROOT / "PATCH_VERSION"
    ).read_text(encoding="utf-8").strip(), (
        "RELEASE_STATUS patch_version must match PATCH_VERSION"
    )
    assert set(release_gate) == {"status", "patch_version"}, (
        "RELEASE_STATUS contains unsupported fields"
    )
    assert "Publication is blocked by RELEASE_STATUS" in build
    for release_control in (
        "package-test:",
        "release-candidate:",
        "starry-release-candidate-${{ needs.resolve.outputs.release_tag }}",
        "sha256sum --check SHA256SUMS",
        "Create or verify immutable annotated release tag",
        '--jq .object.sha)" = "$GITHUB_SHA"',
        "id: final-image",
        "scripts/write_release_summary.py",
        "STARRY-RELEASE-SUMMARY.json",
        "--image-linux-amd64-digest",
        "actions/attest-build-provenance@96278af6caaf10aea03fd8d33a09a777ca52d62f",
        "actions/attest-sbom@4651f806c01d8637787e274ac3bdf724ef169f34",
        "attestations: write",
        "id-token: write",
        "context: candidate/docker",
    ):
        assert release_control in build, (
            f"release workflow is missing immutable candidate control: {release_control}"
        )
    assert build.index("\n  release-candidate:") < build.index("\n  publish:"), (
        "publication must consume an already assembled candidate"
    )
    assert build.index("Create or verify immutable annotated release tag") < build.index(
        "name: Push the final linux/amd64 image"
    ), "the exact immutable source tag must exist before image publication"
    assert build.index("name: Record image and OpenAPI/schema publication summary") < build.index(
        "name: Sign build provenance for every candidate subject"
    ), "the published digest summary must be covered by release attestations"
    assert '--target "$GITHUB_SHA"' not in build, (
        "GitHub Release creation must consume the pre-verified annotated tag"
    )
    for container_release_metadata in (
        "org.opencontainers.image.url=https://github.com/${{ github.repository }}/releases/tag/${{ needs.resolve.outputs.release_tag }}",
        "org.opencontainers.image.documentation=https://github.com/${{ github.repository }}/blob/${{ github.sha }}/docs/container/CONTAINER.md",
        "the same image bundles HBBR with compatible Relay data forwarding plus active probes and load telemetry",
        "account/API services and MMDB data are not included",
        "Recommended Docker deployment: https://github.com/${GITHUB_REPOSITORY}/wiki/Docker-Deployment",
        "Single-host Compose asset: https://github.com/${GITHUB_REPOSITORY}/releases/download/${RELEASE_TAG}/compose.yaml",
        "Control Agent sidecar example: https://github.com/${GITHUB_REPOSITORY}/blob/${RELEASE_TAG}/examples/control-agent/compose.yaml",
    ):
        assert container_release_metadata in build, (
            f"release metadata is missing Docker guidance: {container_release_metadata}"
        )
    for documentation_control in (
        "python3 scripts/export_docs.py release",
        '--ref "$GITHUB_SHA" --repository "$GITHUB_REPOSITORY"',
        "examples config docs/examples",
        'notes_file="candidate/release-assets/RELEASE-NOTES-patch-v${PATCH_VERSION}.md"',
        'chinese_notes="docs/releases/RELEASE-NOTES-patch-v${PATCH_VERSION}.zh-CN.md"',
        'cat "$notes_file" >> release-notes.md',
        "python3 -m unittest discover -s scripts -p 'test_docs.py'",
    ):
        assert documentation_control in build, (
            f"release workflow is missing classified documentation support: {documentation_control}"
        )

    action_count = 0
    for path in paths:
        text = path.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), 1):
            match = USES.search(line)
            if not match:
                continue
            action_count += 1
            ref = match.group(1)
            assert FULL_COMMIT.fullmatch(ref), (
                f"{path.relative_to(ROOT)}:{line_number}: external action ref "
                f"must be a full lowercase commit SHA, got {ref!r}"
            )
        assert not re.search(r"(?m)^\s+toolchain:\s*(stable|beta|nightly)\s*$", text), (
            f"{path.relative_to(ROOT)} uses a moving Rust toolchain"
        )

    assert action_count > 0, "workflow action pin check did not inspect any actions"
    print(
        f"workflow inputs OK: {action_count} action uses pinned, "
        f"Rust {toolchain_version}, cross driver/images, and CI scanners fixed, "
        f"publication {release_gate['status'].lower()}"
    )


if __name__ == "__main__":
    main()
