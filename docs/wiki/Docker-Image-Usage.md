# Docker image usage

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Image-Usage)

The authoritative container manual is distributed with the source and Release
assets as [`CONTAINER.md`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CONTAINER.md).
This page provides the package-oriented index.

## Image identity

```text
ghcr.io/q1ngyang/rustdesk-server-starry
```

Supported platforms:

- `linux/amd64`

ARM remains a best-effort source-compatibility target. patch-v1.2.0 does not
promise or publish a `linux/arm64` image.

The image contains patched `hbbs`, unmodified upstream `hbbr`, and unmodified
`rustdesk-utils`. Starry functionality exists only in HBBS. The recommended
deployment makes that boundary visible by running HBBR from the official
`rustdesk/rustdesk-server` image.

No API server or MMDB data is embedded.

## Tags and digests

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0
```

Use a version tag for deliberate upgrades and a digest for immutable
reproduction. `latest` is a moving reference.

## Commands

```sh
docker run --rm IMAGE hbbs --help
docker run --rm IMAGE hbbr --help
docker run --rm IMAGE rustdesk-utils --help
```

Replace `IMAGE` with a complete tag or digest. `hbbs` is the default image
command and reads `/root/starry/config.yaml` when started by the provided
Compose example.

## Storage contract

Mount one persistent directory at `/root`. It contains:

- server identity keys;
- the RustDesk database;
- logs and runtime state;
- `starry/config.yaml` and its generated example; and
- MMDB files referenced by relative paths.

Back up the directory as a unit. A container replacement must not generate a
new identity accidentally.

## Start here

- [Container manual](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CONTAINER.md)
- [Single-host Compose](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [Compose ENV](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/.env.example)
- [Docker Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment)
- [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback)

For a linked package, GitHub documents that the package page displays
[repository information such as the README](https://docs.github.com/en/packages/learn-github-packages/connecting-a-repository-to-a-package).
Because GHCR's supported package metadata does not include a separate full
README annotation, the container-specific manual is kept as an independent,
versioned document and exposed through repository, Release, and OCI
documentation links.
