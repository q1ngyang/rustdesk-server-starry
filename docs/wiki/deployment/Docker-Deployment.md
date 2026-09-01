# Docker deployment reference

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)

Docker Compose on `linux/amd64` is the recommended Starry deployment. If this
is your first self-hosted RustDesk server, follow the complete
[single-host walkthrough](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started)
first. This page is the day-two reference for the supplied Compose files,
storage, networking, configuration, and change procedure.

## Supplied deployments

| Scenario | Compose and environment | Use it for |
| --- | --- | --- |
| Single host | `examples/compose.yaml`, `examples/.env.example` | Starry HBBS plus HBBR on one server; recommended starting point |
| Centre | `examples/center/compose.yaml`, `examples/center/.env.example` | One HBBS centre with its local HBBR and remote Relay nodes |
| Relay-only node | `examples/relay/compose.yaml`, `examples/relay/.env.example` | One remote HBBR |
| Control Agent | `examples/control-agent/compose.yaml`, `examples/control-agent/.env.example` | HBBS, HBBR, and the optional private management sidecar |

Every supplied HBBR service uses the **same pinned Starry image tag** as HBBS.
Its byte-forwarding path comes from the pinned RustDesk Server revision;
Starry adds bounded quality probes and load/version telemetry. Do not substitute a
separately updated official image in these examples.

## Single-host service layout

```text
Internet
  ├─ 21115/TCP ───────────────> HBBS NAT test
  ├─ 21116/TCP+UDP ───────────> Starry HBBS
  ├─ 21117/TCP ───────────────> bundled HBBR (upstream relay data path)
  └─ 443/TCP ──> Nginx
                   ├─ /ws/id ─────> 127.0.0.1:21118 (HBBS)
                   └─ /ws/relay ──> 127.0.0.1:21119 (HBBR)

Host data directory ──────────> /root in both containers
```

The examples use host networking. This preserves the client addresses needed
by Geo rules and exposes the RustDesk listeners directly on the Linux host.
They are not Docker Desktop examples.

## Persistence contract

Mount one durable host directory at `/root` in both HBBS and HBBR. Back up the
directory as one unit before upgrades.

| Path below `/root` | Meaning | Required action |
| --- | --- | --- |
| `id_ed25519` | Server private identity | Keep secret; never copy to clients or Relay-only nodes |
| `id_ed25519.pub` | Client/server public key | Back up; distribute its one-line value to clients and Relay-only nodes |
| `db_v2.sqlite3` | RustDesk server state | Back up consistently |
| `starry/config.yaml` | Active Starry configuration candidate | Version privately and validate every change |
| `starry/config.example.yaml` | Generated local reference | Reference only; do not assume it is active |
| `mmdb/*.mmdb` | Operator-provided Geo databases | Keep current and respect the data licence |

Container replacement must not create a new identity accidentally. If
`id_ed25519` disappears, stop and restore the data directory before clients
are reconfigured.

## Required and recommended settings

In `.env`:

| Setting | Requirement |
| --- | --- |
| `STARRY_IMAGE` | Required; keep the GHCR image unless using a verified private mirror |
| `STARRY_VERSION` | Required; pin an exact release, not `latest`, in production |
| `STARRY_DATA_DIR` or `STARRY_PERSIST_ROOT` | Required; use a dedicated, backed-up absolute host path |
| `RUSTDESK_LOG_LEVEL` | Keep `info`; use `debug` only temporarily |

For a complete single host, begin with
[`config/config.single-host.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.single-host.yaml).
Replace the example hostname and keep these commissioning defaults:

- `secure_tcp.mode: auto`;
- `geo.enabled: false` until the required MMDB files exist;
- `websocket_signal.enabled: false` until TLS and both proxy paths work; and
- `connection_auth.mode: off` until a compatible token issuer is deployed and
  audited.

The configuration parser rejects unknown fields and invalid dependencies as a
whole candidate. Check HBBS logs after every restart or managed activation.

## Start or update the stack

```sh
docker compose --env-file .env -f compose.yaml config
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 120 hbbs hbbr
```

Confirm that HBBS and HBBR resolved to the same image:

```sh
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}}'
docker inspect rustdesk-hbbr --format '{{.Config.Image}}'
```

Static Compose validation and a running status are partial checks. Complete a
real native session and a real Relay session before declaring the deployment
usable.

## Firewall and reverse proxy

Open `21115/TCP`, `21116/TCP`, `21116/UDP`, and `21117/TCP` for normal native
operation. Open `80/TCP` for certificate issuance/redirect and `443/TCP` when
WSS is used. Do not expose `21118/TCP` or `21119/TCP`; only the local trusted
Nginx process should reach them. Keep optional Control Agent `21120/TCP` on
loopback or a private management network.

The complete single-host Nginx files are:

- `examples/nginx/single-host.bootstrap.conf` for initial certificate setup;
- `examples/nginx/single-host.example.conf` for `/ws/id` and `/ws/relay` after
  the certificate exists.

See [Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS)
for separate centre/Relay hosts and verification rules.

## API services

No account/API server is included in the image or Compose examples. Compatible
third-party APIs may be deployed separately; the recommended implementation is
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku).
Read [Account/API Integration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration)
before adding it. The joint-deployment tutorial link will be added when that
Kessoku Wiki page is ready.

## Change and rollback procedure

1. Back up the data directory, `.env`, Compose file, Starry configuration, and
   TLS material.
2. Change one layer at a time: image, Starry configuration, proxy, or API.
3. Run static validation before recreation.
4. Inspect HBBS/HBBR logs and confirm the accepted configuration.
5. Repeat native, forced-Relay, Geo, and WSS tests for every enabled path.
6. If acceptance fails, restore the previous pinned image and configuration
   without deleting the persistent data.

Use [Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification)
and [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback)
for the full acceptance and recovery checklists.
