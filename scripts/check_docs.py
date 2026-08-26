#!/usr/bin/env python3
"""Validate the repository documentation set without network access."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from export_docs import DOCS, LINK, ROOT, WIKI, local_target, release_documents, wiki_pages


def managed_markdown() -> list[Path]:
    # Include untracked drafts during local review, but never inspect ignored
    # private plans, deployment data, or an upstream checkout.
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"],
        cwd=ROOT, check=True, capture_output=True,
    )
    return sorted({
        ROOT / name.decode("utf-8")
        for name in result.stdout.split(b"\0")
        if name and (ROOT / name.decode("utf-8")).is_file()
    })


def paired_documents(documents: list[Path]) -> list[tuple[Path, Path]]:
    root_pair = (ROOT / "README.md", DOCS / "project/README.zh-CN.md")
    pairs = {root_pair}
    for document in documents:
        if document in root_pair or document == WIKI / "_Sidebar.md":
            continue
        if document.is_relative_to(DOCS / "archive"):
            continue  # Historical records retain their original language.
        if document.is_relative_to(WIKI):
            english = document.with_name(document.name.removeprefix("ZH-CN-"))
            chinese = english.with_name(f"ZH-CN-{english.name}")
        else:
            english = document.with_name(document.name.replace(".zh-CN.md", ".md"))
            chinese = english.with_suffix(".zh-CN.md")
        pairs.add((english, chinese))
    return sorted(pairs)


def main() -> int:
    errors: list[str] = []
    documents = managed_markdown()
    pairs = paired_documents(documents)
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    try:
        pages = wiki_pages()
    except ValueError as error:
        print(f"documentation error: {error}", file=sys.stderr)
        return 1

    for document in documents:
        if document != ROOT / "README.md" and not document.is_relative_to(DOCS):
            errors.append(f"Markdown belongs under docs/: {document.relative_to(ROOT)}")
    for document in release_documents():
        if not document.is_file():
            errors.append(f"missing release document: {document.relative_to(ROOT)}")

    for english, chinese in pairs:
        if not english.is_file():
            errors.append(f"missing English document: {english.relative_to(ROOT)}")
        if not chinese.is_file():
            errors.append(f"missing Chinese document: {chinese.relative_to(ROOT)}")

    for document in documents:
        if not document.is_file():
            continue
        text = document.read_text(encoding="utf-8")
        if not text.startswith("#"):
            errors.append(f"document has no leading heading: {document.relative_to(ROOT)}")
        if sum(line.startswith("```") for line in text.splitlines()) % 2:
            errors.append(f"unbalanced code fence: {document.relative_to(ROOT)}")
        for match in LINK.finditer(text):
            target = local_target(document, match.group(1), pages)
            if target is None:
                continue
            candidates = [target]
            if not target.suffix:
                candidates.append(target.with_suffix(".md"))
            if not any(candidate.exists() for candidate in candidates):
                errors.append(
                    "broken local link in "
                    f"{document.relative_to(ROOT)}: {match.group(1)}"
                )

    for path in (
        ROOT / "README.md",
        DOCS / "project/README.zh-CN.md",
        ROOT / "examples/.env.example",
        ROOT / "examples/center/.env.example",
        ROOT / "examples/relay/.env.example",
        ROOT / "examples/control-agent/.env.example",
    ):
        if not path.is_file() or f"patch-v{patch_version}" not in path.read_text(encoding="utf-8"):
            errors.append(
                f"current patch version is missing from {path.relative_to(ROOT)}"
            )

    control_env = (ROOT / "examples" / "control-agent" / ".env.example").read_text(
        encoding="utf-8"
    )
    if "STARRY_PERSIST_ROOT=./persist" not in control_env:
        errors.append("Control Agent environment example has no unified persistence root")

    control_compose = (
        ROOT / "examples" / "control-agent" / "compose.yaml"
    ).read_text(encoding="utf-8")
    for required in (
        "${STARRY_PERSIST_ROOT:-./persist}/auth/secrets",
        "${STARRY_PERSIST_ROOT:-./persist}/auth/cache",
        "${STARRY_PERSIST_ROOT:-./persist}/control/secrets",
        "${STARRY_PERSIST_ROOT:-./persist}/control/shared",
        "${STARRY_PERSIST_ROOT:-./persist}/control/state",
        "target: /var/lib/starry-auth",
        "target: /run/secrets/starry-control-shared",
    ):
        if required not in control_compose:
            errors.append(f"Control Agent Compose is missing persistence boundary: {required}")
    for deprecated in ("source: ./data/", "source: ./secrets"):
        if deprecated in control_compose:
            errors.append(f"Control Agent Compose retains split host path: {deprecated}")

    # Docker examples intentionally use one exact Starry release for both HBBS
    # and its bundled, unmodified HBBR. A separate official image can drift
    # independently and recreate the compatibility problem these examples
    # are designed to avoid.
    docker_examples = {
        ROOT / "examples" / "compose.yaml": 2,
        ROOT / "examples" / "center" / "compose.yaml": 2,
        ROOT / "examples" / "relay" / "compose.yaml": 1,
        ROOT / "examples" / "control-agent" / "compose.yaml": 3,
    }
    for compose, expected_starry_services in docker_examples.items():
        text = compose.read_text(encoding="utf-8")
        if "rustdesk/rustdesk-server" in text or "RUSTDESK_SERVER_IMAGE" in text:
            errors.append(
                f"Docker example uses a separately versioned official image: {compose.relative_to(ROOT)}"
            )
        if text.count("image: ${STARRY_IMAGE") != expected_starry_services:
            errors.append(
                "Docker example does not use one Starry release image for all "
                f"services: {compose.relative_to(ROOT)}"
            )

    for env_example in (
        ROOT / "examples" / ".env.example",
        ROOT / "examples" / "center" / ".env.example",
        ROOT / "examples" / "relay" / ".env.example",
        ROOT / "examples" / "control-agent" / ".env.example",
    ):
        text = env_example.read_text(encoding="utf-8")
        if "RUSTDESK_SERVER_IMAGE" in text or "RUSTDESK_SERVER_VERSION" in text:
            errors.append(
                f"Environment example retains a separate HBBR image: {env_example.relative_to(ROOT)}"
            )

    if errors:
        for error in errors:
            print(f"documentation error: {error}", file=sys.stderr)
        return 1

    print(
        f"documentation OK: {len(pairs)} bilingual pairs, "
        f"{len(documents)} Markdown files"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
