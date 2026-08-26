#!/usr/bin/env python3
"""Export classified sources for GitHub Wiki or versioned Release assets.

This command only writes local files. It never commits, pushes, or publishes.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path
from urllib.parse import quote, unquote, urlparse


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
WIKI = DOCS / "wiki"
REPOSITORY = "q1ngyang/rustdesk-server-starry"
LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")


def wiki_pages(directory: Path = WIKI) -> dict[str, Path]:
    """Map stable, flat Wiki filenames to their classified source paths."""
    pages: dict[str, Path] = {}
    names: set[str] = set()
    for page in sorted(directory.rglob("*.md")):
        if page.is_symlink():
            raise ValueError(f"Wiki source must not be a symlink: {page}")
        if page.name.casefold() in names:
            raise ValueError(f"duplicate Wiki filename: {page.name}")
        names.add(page.name.casefold())
        pages[page.name] = page
    for name in ("Home.md", "ZH-CN-Home.md", "_Sidebar.md"):
        if name not in pages:
            raise ValueError(f"missing Wiki entry: {name}")
    return pages


def release_documents() -> list[Path]:
    patch = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
    return [
        ROOT / "README.md",
        DOCS / "project/README.zh-CN.md",
        DOCS / "container/CONTAINER.md",
        DOCS / "container/CONTAINER.zh-CN.md",
        DOCS / "releases/CHANGELOG.md",
        DOCS / "releases/CHANGELOG.zh-CN.md",
        DOCS / f"releases/RELEASE-NOTES-patch-v{patch}.md",
        DOCS / f"releases/RELEASE-NOTES-patch-v{patch}.zh-CN.md",
    ]


def local_target(source: Path, raw: str, pages: dict[str, Path]) -> Path | None:
    target = raw.strip().removeprefix("<").removesuffix(">")
    parsed = urlparse(target)
    path = unquote(parsed.path)
    if parsed.scheme:
        if parsed.netloc.lower() != "github.com":
            return None
        project = f"/{REPOSITORY}"
        if path.rstrip("/") == f"{project}/wiki":
            return pages["Home.md"]
        wiki_prefix = f"{project}/wiki/"
        if path.startswith(wiki_prefix):
            name = f"{path.removeprefix(wiki_prefix)}.md"
            return pages.get(name, WIKI / name)
        for kind in ("blob", "tree"):
            prefix = f"{project}/{kind}/main/"
            if path.startswith(prefix):
                return ROOT / path.removeprefix(prefix)
        # Immutable links into historical release tags are intentionally kept.
        return None
    if not path or target.startswith("//"):
        return None
    if source.is_relative_to(WIKI) and "/" not in path:
        name = path if path.endswith(".md") else f"{path}.md"
        if name in pages:
            return pages[name]
    return (source.parent / path).resolve()


def render(source: Path, pages: dict[str, Path], ref: str, repository: str) -> str:
    """Make links work outside the source tree without changing old tag URLs."""
    wiki_names = {path: Path(name).stem for name, path in pages.items()}
    base = f"https://github.com/{repository}"

    def replace(match: re.Match[str]) -> str:
        raw = match.group(1)
        target = local_target(source, raw, pages)
        if target is None:
            return match.group(0)
        if not target.exists() or not target.is_relative_to(ROOT):
            raise ValueError(f"broken or external local link in {source}: {raw}")
        parsed = urlparse(raw.strip().removeprefix("<").removesuffix(">"))
        if target in wiki_names:
            url = f"{base}/wiki/{wiki_names[target]}"
        else:
            kind = "tree" if target.is_dir() else "blob"
            path = quote(target.relative_to(ROOT).as_posix())
            url = f"{base}/{kind}/{quote(ref, safe='')}/{path}"
        if parsed.query:
            url += f"?{parsed.query}"
        if parsed.fragment:
            url += f"#{parsed.fragment}"
        return match.group(0).replace(f"({raw})", f"({url})")

    return LINK.sub(replace, source.read_text(encoding="utf-8"))


def export(kind: str, output: Path, ref: str, repository: str) -> int:
    pages = wiki_pages()
    sources = list(pages.values()) if kind == "wiki" else release_documents()
    output = output.resolve()
    if output.is_relative_to(DOCS):
        raise ValueError("export outside docs/ so generated copies stay separate")
    if kind == "wiki" and output.exists():
        raise ValueError("Wiki export requires a new directory; existing files are never removed")
    # Validate and render everything before writing any file. Release assets
    # may share an existing directory with binaries, but never overwrite docs.
    rendered = {source.name: render(source, pages, ref, repository) for source in sources}
    for name in rendered:
        if (output / name).exists() or (output / name).is_symlink():
            raise ValueError(f"refusing to overwrite exported document: {output / name}")
    output.mkdir(parents=True, exist_ok=(kind == "release"))
    for name, content in rendered.items():
        with (output / name).open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(content)
    return len(rendered)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("wiki", "release"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ref", default="main", help="Repository revision for source links")
    parser.add_argument("--repository", default=REPOSITORY)
    args = parser.parse_args()
    try:
        count = export(args.kind, args.output, args.ref, args.repository)
    except (OSError, ValueError) as error:
        parser.exit(1, f"documentation export error: {error}\n")
    print(f"exported {count} {args.kind} documents to {args.output}; nothing published")


if __name__ == "__main__":
    main()
