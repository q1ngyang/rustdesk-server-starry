# Project metadata and documentation publication

**English** | [简体中文](PROJECT-METADATA.zh-CN.md)

This file records the maintained GitHub metadata and documentation publication
procedure. Editing it does not authorize automatic publication.

## Repository About

Default English description:

> Unofficial RustDesk Server HBBS overlay with Geo Relay policy, managed MMDB,
> Secure TCP, WebSocket signalling, connection authentication, and bundled
> version-locked HBBR. API not included.

Website:

```text
https://github.com/q1ngyang/rustdesk-server-starry/wiki
```

Topics:

```text
rustdesk
rustdesk-server
hbbs
self-hosted
docker
geoip
websocket
remote-desktop
rust
```

The description deliberately names HBBS, calls the project unofficial, and
states that HBBR is bundled/version-locked while the account API is not.

## GHCR package metadata

`linux/amd64` image description (the workflow also adds the patch version):

> Starry HBBS for RustDesk Server with Geo Relay policy, Secure TCP, WebSocket
> signalling, connection authentication, and optional Control Agent; the same
> image bundles upstream-data-path HBBR with version reporting, while account/API services and MMDB data are
> not included

The build workflow publishes OCI title, source, documentation, version,
revision, licence, and description labels on the image. The documentation URL
points to `docs/container/CONTAINER.md` at the exact build commit. This
also remains valid if a release is rebuilt after a source-path migration;
existing image metadata and historical release tags are not rewritten.

For a linked package, GitHub documents that the package landing page displays
[repository information such as the README](https://docs.github.com/en/packages/learn-github-packages/connecting-a-repository-to-a-package).
The Container Registry's supported package-page annotations provide a source,
description, and licence, but not a separately maintained full README. The
project therefore provides an independent, versioned container manual in
[`CONTAINER.md`](../container/CONTAINER.md), distributes it with Release assets, links it
through OCI metadata, and keeps the root README focused on the project.

## Documentation export and publication

The [classified index](../README.md) covers all documentation. Only
[`docs/wiki/`](../wiki) is exported to the separate GitHub Wiki
repository. Project metadata, container manuals, release notes, technical
contracts, example instructions, and archives are repository documents, not
additional Wiki pages.

[`docs/wiki/_Sidebar.md`](../wiki/_Sidebar.md) supplies the global English/Chinese
index. English is the default home page; every narrative Wiki page has a
`ZH-CN-` counterpart. Directory categories are local organization only: the
exporter uses the original flat filenames, preserving existing Wiki URLs,
and rejects duplicate filenames across categories.

From the repository root, generate a local preview in a new directory:

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts -p 'test_docs.py'
wiki_preview="$(mktemp -d)"
python3 scripts/export_docs.py wiki --output "${wiki_preview}/pages"
```

The exporter does not access GitHub, change Git state, or overwrite an existing
Wiki output directory. Review its output, then obtain publication approval.
After approval, copy only the exported files into a fresh checkout of
`rustdesk-server-starry.wiki.git`, inspect the diff, and commit/push the
approved changes. Do not copy the entire `docs` tree or the nested Wiki source
directories into that checkout; use the flat export instead.
If a page is intentionally removed, review that deletion separately; the
exporter never deletes pages from a Wiki checkout.

## Release and image documentation

The release workflow is prepared to:

- validate all supplied Compose files and bilingual/local documentation links;
- export the English/Chinese project README, container manual, changelog, and
  current patch notes under their unchanged download filenames; relative
  source links are converted to build-commit URLs so flat assets still work;
- attach a complete `examples` plus `config` archive, including the relocated
  instructions under `docs/examples`;
- generate checksums for all downloadable assets; and
- generate the GitHub Release body from the exported, versioned patch notes
  instead of a duplicated hard-coded summary.

Preview the eight standalone Release documents without building or publishing
an image:

```sh
release_preview="$(mktemp -d)"
python3 scripts/export_docs.py release \
  --output "${release_preview}/documents" --ref "$(git rev-parse HEAD)"
```

During an uncommitted review, newly moved paths will only exist locally; the
preview's commit links become usable after the matching revision is committed
and pushed. CI passes its actual `GITHUB_SHA` and `GITHUB_REPOSITORY`.

## Publication gate

After the final diff is reviewed, publication remains a separate explicit
operation:

1. commit and push the approved repository changes;
2. publish the staged Wiki pages;
3. update Repository About description, website, and topics;
4. run the release workflow with publishing enabled, or let the separately
   approved release policy act; and
5. verify the live README, Wiki, Release assets, OCI index description, and
   container documentation link.

No step above should run from an unreviewed working tree.
