# rustdesk-server-starry container image

**English** | [简体中文](CONTAINER.zh-CN.md)

This document is for users who arrive from the GHCR package page and need to
deploy the image. For the project overview and source relationship, see the
main [`README.md`](../../README.md).

Deployment links:

- [GHCR image page](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)
- [Complete beginner walkthrough](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started)
- [Recommended Docker deployment guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment)
- [Single-host Compose example](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [Control Agent sidecar example](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/control-agent/compose.yaml)

## What the image contains

The patch-v1.2.0 release image is built and runtime-tested for `linux/amd64`
from one pinned official RustDesk Server revision plus the Starry HBBS overlay.
ARM remains best-effort source compatibility and is not a promised v1.2.0
image platform.

| Command | Origin | Intended use |
| --- | --- | --- |
| `hbbs` | Official HBBS plus the Starry overlay | ID, rendezvous, signalling, Secure TCP, Geo Relay selection, and optional WebSocket Signal. |
| `hbbr` | Unmodified upstream HBBR | Built from the same pinned upstream revision as HBBS. All supplied examples use this copy from the same Starry image tag to prevent version drift. |
| `rustdesk-utils` | Unmodified upstream utility | Key and database maintenance utilities. |
| `starry-control-agent` | Starry optional Linux management component | Fixed Control API for one local HBBS. It requires mTLS and scoped service JWTs and starts with configuration writes disabled. |

The image does **not** contain an account/API server or any GeoLite2/MMDB
database. The Control Agent is not an account API. Compatible third-party APIs
can be deployed separately; the recommended integration is
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku).
Review each component's licence and keep secrets outside the image.

## Choose a tag

Available release tags use this form:

```text
<official-rustdesk-server-version>-patch-vX.Y.Z
```

For example:

```text
1.1.16-patch-v1.2.0
```

- Use an immutable release tag for normal production deployments.
- Use an image digest when the exact manifest must never change.
- `latest` follows the newest successfully published Starry release and is
  intended for evaluation or operators with a deliberate automatic-update and
  rollback process.

Pull the current documented release:

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0
```

Public GHCR images can be pulled anonymously. Inspect the resolved digest and
platforms before rollout:

```sh
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0 \
  --format '{{json .RepoDigests}}'

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0
```

## Recommended quick start

The repository's [`examples/compose.yaml`](../../examples/compose.yaml) starts:

- Starry HBBS from this image; and
- the unmodified HBBR from the **same pinned Starry image tag**.

On a Linux Docker host:

```sh
mkdir -p /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLo compose.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/compose.yaml
curl -fsSLo .env \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/.env.example

mkdir -p data/starry
curl -fsSLo data/starry/config.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/config/config.single-host.yaml
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
[`config/`](../../config).

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
| `21120` | TCP | optional Control Agent | Private mTLS management API. It is not exposed by the image metadata or normal Compose example; keep it on loopback/private management networking. |
| `443` | TCP | Nginx | Public TLS/WSS endpoint when WebSocket or an HTTPS API is used. |

Open only the paths required by your deployment. WebSocket clients need valid
public TLS endpoints for both `/ws/id` and `/ws/relay`; publishing only one of
them is not a complete WebSocket deployment.

## Configure Starry features

Edit the generated file on the host:

```sh
vi data/starry/config.yaml
```

For initial commissioning, restart HBBS. For later managed changes use the
authenticated Control Agent plan/apply or runtime-reload operation:

```sh
docker restart rustdesk-starry-hbbs
```

If reload reports an error, the whole candidate is rejected and HBBS retains
the previous last-known-good generation. If no valid generation has ever
loaded, HBBS stays on upstream-compatible behaviour. Restore or correct the
file, reload again, and require matching generation/digest/subsystem
acknowledgements; process survival does not make an invalid candidate active.

Use these starting points:

- [`config/config.single-host.yaml`](../../config/config.single-host.yaml): complete
  single-host commissioning profile;
- [`config/config.minimal.yaml`](../../config/config.minimal.yaml): Secure TCP only;
- [`config/config.geo-basic.yaml`](../../config/config.geo-basic.yaml): country-based
  ordered Relay selection;
- [`config/config.geo-advanced.yaml`](../../config/config.geo-advanced.yaml): nested
  city/ASN/ISP rules; and
- [`config/config.websocket.yaml`](../../config/config.websocket.yaml): schema v2
  WebSocket Signal and certificate-verified Relay health; and
- [`config/config.auth-audit.yaml`](../../config/config.auth-audit.yaml): schema v3
  connection-authentication audit canary.

The optional Control Agent has a separate
[`Compose example`](../../examples/control-agent/compose.yaml) and
[`operator guide`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent).
Commission it read-only. Do not publish its listener or HBBS local control on
the public RustDesk ports.

## Run a single command without Compose

Compose is recommended for long-lived services. For inspection or temporary
testing:

```sh
docker run --rm \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0 \
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
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0 \
  hbbs --starry-config=/root/starry/config.yaml
```

Start the bundled, unmodified HBBR from the same Starry image tag and with the
same persistent directory on a single host:

```sh
docker run -d \
  --name rustdesk-hbbr \
  --network host \
  --restart unless-stopped \
  -v /opt/rustdesk-server-starry/data:/root \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0 \
  hbbr -k _
```

## Verify the deployment

Static Compose validation is only the first layer:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
```

Confirm the current Relay allocation pool with authenticated Control Agent
`GET /control/v1/relays`.

Preview a rule decision with two real public egress addresses:

Use authenticated `POST /control/v1/allocations:simulate` with the two
addresses, transport, expected generation, and `explain: true`.

The documentation addresses above are placeholders; replace them with the
public addresses actually observed by HBBS. An allocation simulation previews an
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

When rolling back from patch-v1.2.0 to patch-v1.1.0, restore a schema
`version: 2` (or earlier) configuration before starting the old image.
patch-v1.1.0 does not understand schema v3. For older rollback hops, restore
the schema supported by that release instead of relying on validation fallback.

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
