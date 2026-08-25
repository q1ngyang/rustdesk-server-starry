#!/usr/bin/env python3
"""Validate the repository documentation set without network access."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlparse


ROOT = Path(__file__).resolve().parents[1]
WIKI = ROOT / "docs" / "wiki"
LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")


def paired_documents() -> list[tuple[Path, Path]]:
    pairs = [
        (ROOT / "README.md", ROOT / "README.zh-CN.md"),
        (ROOT / "CONTAINER.md", ROOT / "CONTAINER.zh-CN.md"),
        (ROOT / "CHANGELOG.md", ROOT / "CHANGELOG.zh-CN.md"),
        (
            ROOT / ".github" / "PROJECT-METADATA.md",
            ROOT / ".github" / "PROJECT-METADATA.zh-CN.md",
        ),
    ]
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    pairs.append(
        (
            ROOT / f"RELEASE-NOTES-patch-v{patch_version}.md",
            ROOT / f"RELEASE-NOTES-patch-v{patch_version}.zh-CN.md",
        )
    )
    for english in sorted(WIKI.glob("*.md")):
        if english.name == "_Sidebar.md" or english.name.startswith("ZH-CN-"):
            continue
        pairs.append((english, english.with_name(f"ZH-CN-{english.name}")))
    return pairs


def managed_markdown(pairs: list[tuple[Path, Path]]) -> list[Path]:
    files = {WIKI / "_Sidebar.md"}
    for english, chinese in pairs:
        files.add(english)
        files.add(chinese)
    return sorted(files)


def local_target(source: Path, raw: str) -> Path | None:
    target = raw.strip()
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    target = target.split("#", 1)[0]
    if not target or target.startswith("mailto:"):
        return None
    if target.startswith(("http://", "https://")):
        parsed = urlparse(target)
        if parsed.netloc.lower() != "github.com":
            return None
        project = "/q1ngyang/rustdesk-server-starry"
        path = unquote(parsed.path)
        wiki_prefix = f"{project}/wiki/"
        blob_prefix = f"{project}/blob/main/"
        tree_prefix = f"{project}/tree/main/"
        if path.startswith(wiki_prefix):
            return WIKI / f"{path.removeprefix(wiki_prefix)}.md"
        if path.startswith(blob_prefix):
            return ROOT / path.removeprefix(blob_prefix)
        if path.startswith(tree_prefix):
            return ROOT / path.removeprefix(tree_prefix)
        return None
    # This documentation set does not use optional Markdown link titles. Keep
    # the validator strict so an accidental space in a path is reported.
    return (source.parent / unquote(target)).resolve()


def main() -> int:
    errors: list[str] = []
    pairs = paired_documents()
    patch_version = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()

    for english, chinese in pairs:
        if not english.is_file():
            errors.append(f"missing English document: {english.relative_to(ROOT)}")
        if not chinese.is_file():
            errors.append(f"missing Chinese document: {chinese.relative_to(ROOT)}")

    chinese_wiki = {
        page.name.removeprefix("ZH-CN-")
        for page in WIKI.glob("ZH-CN-*.md")
    }
    english_wiki = {
        page.name
        for page in WIKI.glob("*.md")
        if page.name != "_Sidebar.md" and not page.name.startswith("ZH-CN-")
    }
    for orphan in sorted(chinese_wiki - english_wiki):
        errors.append(f"Chinese Wiki page has no English peer: ZH-CN-{orphan}")

    documents = managed_markdown(pairs)
    for document in documents:
        if not document.is_file():
            continue
        text = document.read_text(encoding="utf-8")
        if not text.startswith("#"):
            errors.append(f"document has no leading heading: {document.relative_to(ROOT)}")
        if sum(line.startswith("```") for line in text.splitlines()) % 2:
            errors.append(f"unbalanced code fence: {document.relative_to(ROOT)}")
        for match in LINK.finditer(text):
            target = local_target(document, match.group(1))
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
        ROOT / "README.zh-CN.md",
        ROOT / "examples/.env.example",
        ROOT / "examples/center/.env.example",
        ROOT / "examples/relay/.env.example",
        ROOT / "examples/control-agent/.env.example",
    ):
        if f"patch-v{patch_version}" not in path.read_text(encoding="utf-8"):
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
