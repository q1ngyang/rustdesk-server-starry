# Docker deployment

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)

Docker Compose on Linux is the recommended Starry deployment. This guide
explains the complete single-host example instead of treating `compose up` as
proof that the service works.

## Architecture

```text
RustDesk clients
    │
    ├── 21116 TCP/UDP ──> Starry HBBS
    │                        │
    │                        └── selects an HBBR
    │
    └── 21117 TCP ─────> official HBBR

Shared persistent directory: identity, database, logs, Starry config, MMDB
```

The sample runs both services on one Linux host using host networking. HBBR is
official and unmodified.

## Files

- [`examples/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [`examples/.env.example`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/.env.example)
- [`config/config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml)
- [`examples/nginx/center.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/center.example.conf), only when HBBS WSS is used
- [`examples/nginx/api.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/api.example.conf), only when the optional API is used

## Prepare

Create a dedicated directory and copy the files:

```sh
sudo install -d -m 0750 -o "$(id -u)" -g "$(id -g)" \
  /opt/rustdesk-server-starry/data
cd /opt/rustdesk-server-starry

cp /path/to/repository/examples/compose.yaml .
cp /path/to/repository/examples/.env.example .env
```

Review the exact images in `.env`. The provided values match
`1.1.16-patch-v1.2.0` to official HBBR `1.1.16`.

## Static validation

```sh
docker compose --env-file .env -f compose.yaml config
docker compose --env-file .env -f compose.yaml config --quiet
```

The first command lets you inspect the merged configuration. Confirm:

- the intended image tags;
- the absolute or expected relative data path;
- exactly one HBBS and one HBBR;
- host networking; and
- no real secret or unintended environment variable is injected.

## First start and identity

```sh
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
```

Confirm `data/id_ed25519` and `data/id_ed25519.pub` are non-empty. The private
file stays on this centre. Store a protected backup before client rollout.

The HBBS health check validates local process reachability and key files only.
It does not validate external routing or a desktop session.

## Configure Starry

For the first connection, copy the minimal template to the generated functional
path:

```sh
cp /path/to/repository/config/config.minimal.yaml \
  data/starry/config.yaml

docker exec rustdesk-starry-hbbs sh -c \
  "test -s /starry/config.yaml"
docker compose --env-file .env -f compose.yaml restart hbbs
```

Then add Geo and WebSocket in separate changes. See
[Configuration Reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference).

## Firewall

For native RustDesk, permit inbound `21116/TCP`, `21116/UDP`, and `21117/TCP`.
Follow the official RustDesk guidance for `21115/TCP` NAT testing. Do not create
a public HTTP endpoint for the loopback management commands.

When WSS is enabled:

- expose `443/TCP` through Nginx;
- keep `21118/TCP` and `21119/TCP` reachable only from the local or explicitly
  trusted proxy network; and
- retain native ports if clients may switch WebSocket off.

## Client setup

Configure both endpoints with the centre ID Server and the exact
`data/id_ed25519.pub` value. Leave Relay Server empty. Leave API and WebSocket
off for the first native test.

See [Client Configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration).

## Verification

Complete all of these, not just `docker compose ps`:

1. HBBS and HBBR logs have no startup error.
2. Both clients register with the self-hosted ID Server.
3. A native control request and desktop session work.
4. A Relay session is exercised when HBBR is part of the acceptance scope.
5. Secure TCP is tested after API login when an API is deployed.
6. Geo decisions are previewed and then verified in a real Relay session.
7. WSS-to-WSS and both mixed directions are tested when WSS is enabled.

Use the commands and evidence rules in
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).

## Back up and upgrade

Back up the entire data directory before changing the image. Pin the current
tag/digest, read both upstream and Starry release notes, pull the target, run
static validation, recreate, and repeat real-client acceptance. Keep the old
image and configuration until verification finishes.

See [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback).
