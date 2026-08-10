# rustdesk-server-starry container image

**English** | [简体中文](CONTAINER.zh-CN.md)

This document is for users who arrive from the GHCR package page and need to
deploy the image. For the project overview and source relationship, see the
main [`README.md`](README.md).

## What the image contains

The multi-architecture image is built for `linux/amd64` and `linux/arm64` from
one pinned official RustDesk Server revision plus the Starry HBBS overlay.

| Command | Origin | Intended use |
| --- | --- | --- |
| `hbbs` | Official HBBS plus the Starry overlay | ID, rendezvous, signalling, Secure TCP, Geo Relay selection, and optional WebSocket Signal. |
| `hbbr` | Unmodified upstream HBBR | Convenience copy built from the same upstream revision. The recommended examples use the official RustDesk Server image for HBBR so the component boundary stays visible. |
| `rustdesk-utils` | Unmodified upstream utility | Key and database maintenance utilities. |

The image does **not** contain an account/API server or any GeoLite2/MMDB
database. Select those independently, review their licences, and keep secrets
outside the image.

## Choose a tag

Available release tags use this form:

```text
<official-rustdesk-server-version>-patch-vX.Y.Z
```

For example:

```text
1.1.16-patch-v1.1.0
```

- Use an immutable release tag for normal production deployments.
- Use an image digest when the exact manifest must never change.
- `latest` follows the newest successfully published Starry release and is
  intended for evaluation or operators with a deliberate automatic-update and
  rollback process.

Pull the current documented release:

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0
```

Public GHCR images can be pulled anonymously. Inspect the resolved digest and
platforms before rollout:

```sh
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  --format '{{json .RepoDigests}}'

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0
```

## Recommended quick start

The repository's [`examples/compose.yaml`](examples/compose.yaml) starts:

- Starry HBBS from this image; and
- unmodified official HBBR from the matching official server release.

On a Linux Docker host:

```sh
mkdir -p /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLO \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.yaml
curl -fsSLo .env \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.env.example

mkdir -p data
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
```

When using a repository checkout instead of Release assets:

```sh
cp examples/.env.example .env
cp examples/compose.yaml compose.yaml
mkdir -p data
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
```

The sample uses Linux host networking. This lets HBBS observe real peer
addresses and exposes the normal RustDesk ports directly on the host. It is not
a Docker Desktop example.

## Persistent data and first start

Both services mount the same host data directory at `/root`. It contains the
server identity, SQLite state, logs, downloaded MMDB files, and Starry
configuration. Back it up as one unit.

The first Starry HBBS start creates:

```text
data/
├── id_ed25519
├── id_ed25519.pub
└── starry/
    ├── config.yaml
    └── config.example.yaml
```

`starry/config.yaml` is deliberately empty. Empty or invalid Starry
configuration disables the entire Starry configuration and leaves HBBS on its
upstream command-line behaviour. Copy only the required sections from
`starry/config.example.yaml`, or start from one of the files under
[`config/`](config/).

Never copy `id_ed25519` to a Relay-only node or into documentation. The public
`id_ed25519.pub` value is the key configured in clients and may be distributed.

## Ports

| Port | Protocol | Process | Purpose |
| --- | --- | --- | --- |
| `21115` | TCP | HBBS | NAT test and loopback-only management commands. Do not proxy the management interface publicly. |
| `21116` | TCP | HBBS | Native signalling, hole-punch coordination, and Secure TCP. |
| `21116` | UDP | HBBS | Registration and heartbeat. |
| `21117` | TCP | HBBR | Native Relay data. |
| `21118` | TCP | HBBS | Plain WebSocket backend for `/ws/id`; restrict it to the trusted reverse proxy. |
| `21119` | TCP | HBBR | Plain WebSocket backend for `/ws/relay`; restrict it to the trusted reverse proxy. |
| `443` | TCP | Nginx | Public TLS/WSS endpoint when WebSocket or an HTTPS API is used. |

Open only the paths required by your deployment. WebSocket clients need valid
public TLS endpoints for both `/ws/id` and `/ws/relay`; publishing only one of
them is not a complete WebSocket deployment.

## Configure Starry features

Edit the generated file on the host:

```sh
vi data/starry/config.yaml
```

Then reload it without replacing the process:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
```

If the reload reports an error, the whole new Starry configuration is rejected.
The previous Starry state is not retained; HBBS returns to upstream behaviour.
Restore or correct the file and reload again. Do not assume a container restart
made an invalid configuration active.

Use these starting points:

- [`config/config.minimal.yaml`](config/config.minimal.yaml): Secure TCP only;
- [`config/config.geo-basic.yaml`](config/config.geo-basic.yaml): country-based
  ordered Relay selection;
- [`config/config.geo-advanced.yaml`](config/config.geo-advanced.yaml): nested
  city/ASN/ISP rules; and
- [`config/config.websocket.yaml`](config/config.websocket.yaml): schema v2
  WebSocket Signal and certificate-verified Relay health.

## Run a single command without Compose

Compose is recommended for long-lived services. For inspection or temporary
testing:

```sh
docker run --rm \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  hbbs --help
```

A manual HBBS process with persistence and host networking:

```sh
mkdir -p /opt/rustdesk-server-starry/data

docker run -d \
  --name rustdesk-starry-hbbs \
  --network host \
  --restart unless-stopped \
  -v /opt/rustdesk-server-starry/data:/root \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  hbbs --starry-config=/root/starry/config.yaml
```

Start official HBBR with the same persistent directory on a single host:

```sh
docker run -d \
  --name rustdesk-hbbr \
  --network host \
  --restart unless-stopped \
  -v /opt/rustdesk-server-starry/data:/root \
  rustdesk/rustdesk-server:1.1.16 \
  hbbr -k _
```

## Verify the deployment

Static Compose validation is only the first layer:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
```

Confirm the current Relay allocation pool:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"
```

Preview a rule decision with two real public egress addresses:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 192.0.2.10 198.51.100.20 native\n' | nc -w 2 127.0.0.1 21115"
```

The documentation addresses above are placeholders; replace them with the
public addresses actually observed by HBBS. A `test-geo` result previews an
allocation decision. It does not create a session or prove that either client
can reach the Relay.

Finish with real clients: native/P2P, native Relay, authenticated Secure TCP,
WSS-to-WSS, and both mixed WSS/native directions as applicable. See
[`Operations and verification`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).

## Upgrade and rollback

1. Pin the currently running tag or digest in `.env`.
2. Back up the complete data directory, especially `id_ed25519`,
   `id_ed25519.pub`, the database, and `starry/config.yaml`.
3. Read the target release notes and configuration migration notes.
4. Pull the target images and run `docker compose config --quiet`.
5. Recreate the services, inspect logs, run management checks, and complete a
   real client session.
6. Keep the previous images and backup until acceptance is complete.

When rolling back from patch-v1.1.0 to patch-v1.0.0, restore a schema
`version: 1` configuration. patch-v1.0.0 does not understand schema v2; leaving
a v2 document in place causes Starry configuration to be rejected and HBBS to
fall back to upstream behaviour.

Do not republish or overwrite an immutable version tag as an upgrade method.

## Common container-specific failures

| Symptom | Check |
| --- | --- |
| `config.yaml` stays empty | This is expected on first start. Copy selected sections from the generated example. |
| Starry features disappear after editing YAML | Read the HBBS reload/startup error. One unknown or invalid field rejects the complete Starry configuration. |
| `test-geo` returns `""` | Check `relay_servers`, official HBBS Relay online state, MMDB availability, and rule Relay names. |
| Native works but WSS does not | Validate both exact Nginx paths, public certificate names, `trusted_proxies`, endpoint coverage, and `websocket-status`. |
| API login works but control times out | Treat API login and HBBS Secure TCP as separate layers; verify `21116/TCP`, the HBBS public key, and `secure_tcp.mode`. |
| Container is healthy but desktop control fails | A process or port check is not a protocol session. Correlate both client logs and server logs for the same attempt. |

For symptom-led diagnosis, see
[`Troubleshooting`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting).

## Licence and disclosure

The image is distributed under AGPL-3.0 and is built from the pinned official
RustDesk Server source plus the Starry overlay. It is an unofficial community
image. It contains no GeoLite2 database. Parts of the code and documentation
were generated or revised with AI assistance and carry no additional warranty.
