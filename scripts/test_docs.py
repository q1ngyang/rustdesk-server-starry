#!/usr/bin/env python3
"""Regression tests for documentation layout and local-only publication exports."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from check_docs import managed_markdown, paired_documents
from export_docs import (
    DOCS, LINK, REPOSITORY, ROOT, WIKI,
    export, local_target, release_documents, render, wiki_pages,
)


class DocumentationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.pages = wiki_pages()
        self.base = f"https://github.com/{REPOSITORY}"

    def test_docs_root_separates_wiki_from_other_categories(self) -> None:
        self.assertEqual(DOCS, ROOT / "docs")
        self.assertEqual(WIKI, DOCS / "wiki")
        self.assertTrue((DOCS / "README.md").is_file())
        self.assertTrue((DOCS / "README.zh-CN.md").is_file())
        self.assertNotIn("README.md", self.pages)
        for source in release_documents():
            self.assertFalse(source.is_relative_to(WIKI))

    def test_classified_pages_keep_their_wiki_slugs(self) -> None:
        source = WIKI / "_Sidebar.md"
        expected = WIKI / "deployment/Docker-Deployment.md"
        self.assertEqual(local_target(source, "Docker-Deployment", self.pages), expected)
        self.assertEqual(
            local_target(source, f"{self.base}/wiki/Docker-Deployment#ports", self.pages),
            expected,
        )
        self.assertEqual(local_target(source, f"{self.base}/wiki", self.pages), WIKI / "Home.md")
        self.assertEqual(local_target(source, "Home.md", self.pages), WIKI / "Home.md")

    def test_moved_root_translation_and_configuration_links_resolve(self) -> None:
        self.assertEqual(
            local_target(ROOT / "README.md", "docs/project/README.zh-CN.md", self.pages),
            DOCS / "project/README.zh-CN.md",
        )
        self.assertEqual(
            local_target(DOCS / "container/CONTAINER.md", "../../examples/compose.yaml", self.pages),
            ROOT / "examples/compose.yaml",
        )

    def test_unresolved_pages_and_orphan_translations_are_detectable(self) -> None:
        target = local_target(WIKI / "Home.md", f"{self.base}/wiki/Missing-Page", self.pages)
        self.assertFalse(target.exists())
        english = WIKI / "deployment/Orphan.md"
        chinese = WIKI / "deployment/ZH-CN-Orphan.md"
        self.assertIn((english, chinese), paired_documents([chinese]))
        reference = DOCS / "reference/orphan.md"
        self.assertIn(
            (reference, reference.with_suffix(".zh-CN.md")),
            paired_documents([reference.with_suffix(".zh-CN.md")]),
        )

    def test_duplicate_wiki_filenames_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "first").mkdir()
            (root / "second").mkdir()
            (root / "first/Topic.md").write_text("# Topic\n", encoding="utf-8")
            (root / "second/topic.md").write_text("# Duplicate\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate Wiki filename"):
                wiki_pages(root)

    def test_missing_wiki_home_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "missing Wiki entry"):
                wiki_pages(Path(directory))

    def test_wiki_export_is_flat_and_contains_only_pages(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "wiki"
            self.assertEqual(export("wiki", output, "main", REPOSITORY), len(self.pages))
            self.assertEqual({path.name for path in output.iterdir()}, set(self.pages))
            self.assertTrue(all(path.is_file() for path in output.iterdir()))
            self.assertNotIn("CONTAINER.md", self.pages)
            self.assertNotIn("PROJECT-METADATA.md", self.pages)
            self.assertNotIn("HBBS-GEO-DEVELOPMENT-PLAN.md", self.pages)

    def test_wiki_export_refuses_existing_or_source_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            marker = output / "keep.txt"
            marker.write_text("untouched", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "new directory"):
                export("wiki", output, "main", REPOSITORY)
            self.assertEqual(marker.read_text(encoding="utf-8"), "untouched")
        with self.assertRaisesRegex(ValueError, "outside docs/"):
            export("wiki", DOCS / "export-must-not-be-created", "main", REPOSITORY)

    def test_release_names_stay_stable_and_links_are_portable(self) -> None:
        patch = (ROOT / "PATCH_VERSION").read_text(encoding="utf-8").strip()
        expected = {
            "README.md", "README.zh-CN.md", "CONTAINER.md", "CONTAINER.zh-CN.md",
            "CHANGELOG.md", "CHANGELOG.zh-CN.md",
            f"RELEASE-NOTES-patch-v{patch}.md", f"RELEASE-NOTES-patch-v{patch}.zh-CN.md",
        }
        self.assertEqual({path.name for path in release_documents()}, expected)
        ref = "a" * 40
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            marker = output / "existing-binary"
            marker.write_bytes(b"preserved binary")
            self.assertEqual(export("release", output, ref, REPOSITORY), 8)
            self.assertEqual({path.name for path in output.glob("*.md")}, expected)
            self.assertEqual(marker.read_bytes(), b"preserved binary")
            readme = (output / "README.md").read_text(encoding="utf-8")
            self.assertIn(f"{self.base}/blob/{ref}/docs/project/README.zh-CN.md", readme)
            for path in output.glob("*.md"):
                text = path.read_text(encoding="utf-8")
                for match in LINK.finditer(text):
                    self.assertTrue(
                        match.group(1).startswith(("https://", "http://", "#", "mailto:")),
                        f"relative link in standalone asset {path.name}: {match.group(1)}",
                    )
            with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
                export("release", output, ref, REPOSITORY)
            self.assertEqual((output / "README.md").read_text(encoding="utf-8"), readme)

    def test_historical_tag_and_third_party_links_are_preserved(self) -> None:
        source = DOCS / "releases/RELEASE-NOTES-patch-v1.2.0.md"
        historical = f"{self.base}/blob/1.1.16-patch-v1.2.0/CONTAINER.md"
        self.assertIsNone(local_target(source, historical, self.pages))
        self.assertIn(historical, render(source, self.pages, "a" * 40, REPOSITORY))
        third_party = "https://github.com/q1ngyang/rustdesk-api-kessoku/wiki"
        self.assertIsNone(local_target(source, third_party, self.pages))

    def test_ignored_private_plan_is_not_managed(self) -> None:
        self.assertNotIn(DOCS / "archive/HBBS-GEO-DEVELOPMENT-PLAN.md", managed_markdown())


if __name__ == "__main__":
    unittest.main()
