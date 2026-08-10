# Getting started

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)

This guide takes a new operator from an empty Linux server to a verified native
RustDesk connection. Optional Geo rules, API login, and WebSocket are added only
after the base path works.

## 1. Choose a deployment

| Method | Use it when | Notes |
| --- | --- | --- |
| Docker Compose | Most Linux servers | Recommended. Easiest to reproduce, back up, and roll back. |
| DEB packages | Debian or Ubuntu with native systemd management | HBBS, HBBR, and utilities are separate packages. |
| Standalone binaries | You manage paths and services yourself | Useful on other Linux distributions and Windows. |
| Source overlay build | You are developing or auditing the patch | Not the shortest production path. |

The rest of this page uses Docker Compose.

## 2. Prepare the server

You need:

- a supported `linux/amd64` or `linux/arm64` host;
- Docker Engine with the Compose plugin;
- a persistent directory with enough space for keys, logs, SQLite state, and
  MMDB files;
- a public IP or DNS name reachable by both clients;
- host firewall control; and
- valid backups before replacing an existing RustDesk Server.

Base native ports:

| Port | Required for the first native test? |
| --- | --- |
| `21115/TCP` | Yes for the standard NAT test, but never expose the Starry management interface through an HTTP proxy. |
| `21116/TCP` | Yes. |
| `21116/UDP` | Yes. |
| `21117/TCP` | Yes when the session needs Relay. |
| `21118/TCP`, `21119/TCP`, `443/TCP` | Not until WebSocket is enabled. |

## 3. Download the examples

```sh
sudo mkdir -p /opt/rustdesk-server-starry
sudo chown "$(id -u):$(id -g)" /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLO \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.yaml
curl -fsSLo .env \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.env.example
mkdir -p data
```

Review `.env`. The release tag should be immutable for production. The official
HBBR version should match the official prefix of the Starry release.

## 4. Validate and start

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
```

If static validation fails, stop and correct that error. Do not substitute a
different Compose file or remove required values until the cause is understood.

Inspect startup evidence:

```sh
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
test -s data/id_ed25519
test -s data/id_ed25519.pub
test -f data/starry/config.yaml
test -f data/starry/config.example.yaml
```

The functional `data/starry/config.yaml` is empty on first start. That is
normal.

## 5. Enable only Secure TCP first

Copy the minimal configuration:

```sh
cp \
  /path/to/repository/config/config.minimal.yaml \
  data/starry/config.yaml
```

Or write the equivalent content:

```yaml
version: 1

secure_tcp:
  mode: auto
```

Reload and read the result:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
```

One invalid or unknown field rejects the complete Starry document. Correct the
reported field instead of assuming a partial configuration is active.

## 6. Configure two clients

On both clients, open **Settings → Network** and set:

| Field | First test value |
| --- | --- |
| ID Server | The HBBS public DNS name or IP. |
| Key | The complete one-line content of `data/id_ed25519.pub`. |
| Relay Server | Leave empty so HBBS can allocate it. |
| API Server | Leave empty for the first native test. |
| Use WebSocket | Off. |

Restart or reconnect the clients if their current version does not immediately
re-register after a network-setting change.

## 7. Verify the native path

Test in this order:

1. both clients show IDs from the self-hosted server;
2. a control request reaches the remote client;
3. a desktop session transfers input and video in both directions;
4. if the connection is P2P, temporarily test a Relay path from suitable
   networks rather than assuming HBBR works; and
5. inspect logs for the same attempt if any layer fails.

The public key must match the centre HBBS key. API credentials, a Relay public
key, and a control-session identifier are not replacements for that value.

## 8. Back up before adding features

Back up at least:

```text
data/id_ed25519
data/id_ed25519.pub
data/db_v2.sqlite3 (when present)
data/starry/config.yaml
```

Keep the private key confidential. Losing it changes the public key clients
trust; leaking it requires identity rotation.

## 9. Add features one at a time

Recommended order:

1. [GEO Rules: Basics](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics)
2. [Multi-Node Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment), if needed
3. a separately selected API, if account features are required
4. [Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS)
5. schema v2 and optional WebSocket Signal
6. [Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification)

After each change, preserve the last known-good configuration and complete the
checks for the layer you changed.
