# Project metadata publication draft

**English** | [简体中文](PROJECT-METADATA.zh-CN.md)

This file records the external GitHub values that accompany the documentation
revision. It is a review draft, not an instruction to publish automatically.

## Repository About

Current description:

> Lightweight overlay for official RustDesk Server: ordered GEO Relay routing,
> MMDB updates, and native HBBS Secure TCP compatibility.

Proposed default English description:

> Unofficial RustDesk Server HBBS overlay with Geo Relay policy, managed MMDB,
> Secure TCP, WebSocket signalling, connection authentication, and bundled
> version-locked HBBR. API not included.

Proposed website:

```text
https://github.com/q1ngyang/rustdesk-server-starry/wiki
```

Proposed topics:

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

Proposed `linux/amd64` image description:

> Starry HBBS for RustDesk Server with Geo Relay policy, Secure TCP, WebSocket
> signalling, connection authentication, and optional Control Agent; the same
> image bundles unmodified HBBR, while account/API services and MMDB data are
> not included

The build workflow publishes OCI title, source, documentation, version,
revision, licence, and description labels on the image. The documentation URL points directly to
`CONTAINER.md`.

For a linked package, GitHub documents that the package landing page displays
[repository information such as the README](https://docs.github.com/en/packages/learn-github-packages/connecting-a-repository-to-a-package).
The Container Registry's supported package-page annotations provide a source,
description, and licence, but not a separately maintained full README. The
project therefore provides an independent, versioned container manual in
[`CONTAINER.md`](../CONTAINER.md), distributes it with Release assets, links it
through OCI metadata, and keeps the root README focused on the project.

## Wiki source and publication

The reviewed Wiki source is staged under [`docs/wiki/`](../docs/wiki). GitHub
Wiki is a separate Git repository, so these files do not update the live Wiki
until they are copied and pushed to `rustdesk-server-starry.wiki.git` after
approval.

`_Sidebar.md` supplies a global English/Chinese index. English is the default
home page; every narrative page has a `ZH-CN-` counterpart.

## Release and image documentation

The release workflow is prepared to:

- validate all supplied Compose files and bilingual/local documentation links;
- attach the English/Chinese README, container manual, changelog, and patch
  release notes;
- attach a complete `examples` plus `config` archive;
- generate checksums for all downloadable assets; and
- generate the GitHub Release body from the versioned patch notes instead of a
  duplicated hard-coded summary.

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
