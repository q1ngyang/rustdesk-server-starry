# Getting started: complete single-host Docker deployment

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)

This walkthrough starts with an empty Linux server and finishes with Starry
HBBS, the bundled HBBR, Secure TCP, Geo Relay rules, TLS, and WebSocket
signalling working on one host. It deliberately excludes an account/API server.

The commands assume Debian or Ubuntu on `linux/amd64`, a user with `sudo`, and
one public name such as `rustdesk.example.com`. Replace that name everywhere.

## 1. Understand the result

```text
RustDesk desktop clients
  ├─ 21115/TCP        NAT type test
  ├─ 21116/TCP+UDP    registration, signalling, hole punching, Secure TCP
  ├─ 21117/TCP        native Relay data
  └─ 443/TCP          optional WSS /ws/id and /ws/relay through Nginx

Nginx on the same host
  ├─ /ws/id    -> 127.0.0.1:21118 (HBBS)
  └─ /ws/relay -> 127.0.0.1:21119 (HBBR)
```

Both containers use the same immutable Starry image tag. The HBBR binary in
that image is unmodified upstream code built from the same pinned source as the
release, so HBBS and HBBR cannot drift because another image was updated.

## 2. Prepare DNS and the host

Create an `A` record for `rustdesk.example.com` pointing to the server's public
IPv4 address. Add an `AAAA` record only when IPv6 is configured and reachable.
Wait until the name resolves to the server before requesting a certificate:

```sh
getent ahosts rustdesk.example.com
```

Install Docker Engine and the Compose plugin from the
[official Docker guide](https://docs.docker.com/engine/install/). Then verify:

```sh
docker version
docker compose version
```

Do not continue until both commands succeed. If this server already runs a
RustDesk Server, back up its identity keys and stop the old services first;
never run two HBBS instances on the same ports.

## 3. Download the deployment files

```sh
sudo mkdir -p /opt/rustdesk-server-starry/data/starry
sudo chown -R "$(id -u):$(id -g)" /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLo compose.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/compose.yaml
curl -fsSLo .env \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/.env.example
curl -fsSLo data/starry/config.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/config/config.single-host.yaml
```

The important host paths are:

| Host path | Container path | Keep and back up? |
| --- | --- | --- |
| `/opt/rustdesk-server-starry/data` | `/root` | **Yes.** Contains identity keys, SQLite state, Starry configuration, and MMDB files. |
| `/opt/rustdesk-server-starry/compose.yaml` | n/a | Yes. Records the service definition. |
| `/opt/rustdesk-server-starry/.env` | n/a | Yes, privately. Records the pinned image and local paths. |
| `/etc/letsencrypt` | Nginx host path | **Yes.** Managed separately from the containers. |

Never publish `data/id_ed25519`, `.env`, or private TLS keys.

## 4. Review the required settings

Open `.env` in an editor. For the first deployment, these are the required and
recommended values:

| Variable | Requirement | Recommended value |
| --- | --- | --- |
| `STARRY_IMAGE` | Required | `ghcr.io/q1ngyang/rustdesk-server-starry` |
| `STARRY_VERSION` | Required | Exact release, currently `1.1.16-patch-v1.2.2`; do not use `latest` in production. |
| `STARRY_DATA_DIR` | Required | `/opt/rustdesk-server-starry/data` is clearer than a relative path under service managers. |
| `RUSTDESK_LOG_LEVEL` | Recommended | `info`; use `debug` only temporarily during diagnosis. |
| Restart policy and container names | Optional | Keep the example values unless they conflict locally. |

Next edit `data/starry/config.yaml`:

1. replace every `rustdesk.example.com` with the real public name;
2. keep `secure_tcp.mode: auto`;
3. keep `websocket_signal.enabled: false` until Nginx and the certificate work;
4. keep `geo.enabled: false` until a lawful MMDB file is present; and
5. keep `connection_auth.mode: off` because this walkthrough has no API/token
   issuer.

The file uses configuration structure version `3`, which is required for the
current patch-v1.2.2 feature set. Unknown fields reject the whole candidate;
never guess a key name.

## 5. Configure the firewall

Allow the SSH port you actually use **before** changing a remote firewall.
Then permit these public ports:

| Port | Public? | Purpose |
| --- | --- | --- |
| `21115/TCP` | Yes | RustDesk NAT type test. |
| `21116/TCP` | Yes | Registration, signalling, and Secure TCP. |
| `21116/UDP` | Yes | ID registration and hole punching. |
| `21117/TCP` | Yes | Native HBBR Relay data. |
| `80/TCP` | Yes | Certificate issuance and HTTPS redirect. |
| `443/TCP` | Yes when WSS is enabled | Public TLS for `/ws/id` and `/ws/relay`. |
| `21118/TCP`, `21119/TCP` | **No** | Plain WebSocket backends; only local Nginx should reach them. |
| `21120/TCP` | **No** for this guide | Optional Control Agent management port; keep on loopback/private management networks. |

Example with UFW (replace the SSH rule if your SSH port is not `22`):

```sh
sudo ufw allow 22/tcp
sudo ufw allow 21115/tcp
sudo ufw allow 21116/tcp
sudo ufw allow 21116/udp
sudo ufw allow 21117/tcp
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw deny 21118/tcp
sudo ufw deny 21119/tcp
sudo ufw enable
sudo ufw status numbered
```

Cloud firewalls or security groups need the same public rules. Do not assume a
host rule also changed the provider firewall.

## 6. Start HBBS and HBBR

```sh
cd /opt/rustdesk-server-starry
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 120 hbbs hbbr
```

The first command is only a static check. Confirm the runtime files too:

```sh
test -s data/id_ed25519
test -s data/id_ed25519.pub
test -s data/starry/config.yaml
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}}'
docker inspect rustdesk-hbbr --format '{{.Config.Image}}'
```

The two `docker inspect` commands must print the same Starry tag. Logs should
say that the Starry configuration loaded; a running container alone is not
proof that the candidate was accepted.

## 7. Configure the RustDesk clients

On both desktop clients, open **Settings → Network** and enter:

| RustDesk field | Value |
| --- | --- |
| ID Server | `rustdesk.example.com` |
| Key | The complete one-line content of `data/id_ed25519.pub` |
| Relay Server | Leave empty so HBBS can allocate the configured Relay. |
| API Server | Leave empty in this Starry-only deployment. |
| Use WebSocket | Off for the first test. |

Read the public key without exposing the private key:

```sh
cat /opt/rustdesk-server-starry/data/id_ed25519.pub
```

Restart or reconnect both clients after changing network settings.

## 8. Verify native and Relay sessions

Verify in order:

1. both clients receive IDs from this server;
2. a connection request reaches the remote client;
3. keyboard, pointer, and video work in a real session;
4. test from networks that require Relay, or enable the client's
   always-use-Relay option when the installed client provides it; and
5. correlate both clients and HBBR logs for the same attempt.

```sh
docker compose --env-file .env -f compose.yaml logs --since 10m hbbs hbbr
```

A successful P2P session does not test HBBR. An open TCP port does not test a
RustDesk protocol session.

## 9. Add MMDB and enable Geo rules

The image contains no GeoLite2 database. Obtain Country/City/ASN MMDB files
from a lawful source and comply with its licence. For manual files:

```sh
mkdir -p /opt/rustdesk-server-starry/data/mmdb
cp /path/to/GeoLite2-Country.mmdb \
  /opt/rustdesk-server-starry/data/mmdb/GeoLite2-Country.mmdb
```

The host `data/mmdb` directory appears as `/root/mmdb` in HBBS, matching the
relative paths in the template. Country matching needs the Country database or
a City database containing country data. City rules need City; ASN and ISP
rules need ASN.

Edit `data/starry/config.yaml` and set:

```yaml
geo:
  enabled: true
```

Keep the template's catch-all rule. If you have authorised direct HTTPS MMDB
URLs, fill the corresponding `mmdb.*.url`, set `update_on_start: true`, and
retain the weekly update interval. Otherwise leave URLs empty and replace the
files manually.

Restart only HBBS, then inspect acceptance and MMDB warnings:

```sh
docker compose --env-file .env -f compose.yaml restart hbbs
docker compose --env-file .env -f compose.yaml logs --tail 150 hbbs
```

Geo rules are ordered priorities, not load balancing. The first matching rule
and first eligible Relay win.

## 10. Install Nginx and obtain TLS

Install Nginx and Certbot:

```sh
sudo apt update
sudo apt install -y nginx certbot python3-certbot-nginx
```

Download the temporary HTTP site, replace the example name, enable it, and
request the certificate:

```sh
cd /opt/rustdesk-server-starry
curl -fsSLo nginx-bootstrap.conf \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/nginx/single-host.bootstrap.conf
sudo cp nginx-bootstrap.conf /etc/nginx/sites-available/rustdesk-starry.conf
sudo editor /etc/nginx/sites-available/rustdesk-starry.conf
sudo ln -sfn /etc/nginx/sites-available/rustdesk-starry.conf \
  /etc/nginx/sites-enabled/rustdesk-starry.conf
sudo nginx -t
sudo systemctl reload nginx
sudo certbot --nginx -d rustdesk.example.com
```

Now download the final configuration, replace the domain and certificate paths,
then test before reloading:

```sh
curl -fsSLo nginx-starry.conf \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/nginx/single-host.example.conf
sudo cp nginx-starry.conf /etc/nginx/sites-available/rustdesk-starry.conf
sudo editor /etc/nginx/sites-available/rustdesk-starry.conf
sudo nginx -t
sudo systemctl reload nginx
```

If `nginx -t` fails, stop and correct the exact error. Do not disable
certificate validation or expose `21118/21119` as a workaround.

## 11. Enable and verify WebSocket signalling

Edit `data/starry/config.yaml` and set:

```yaml
websocket_signal:
  enabled: true
```

The template already trusts only local Nginx and maps the exact Relay to
`wss://rustdesk.example.com/ws/relay`. Desktop clients normally send no
`Origin`; keep `allowed_origins: []` unless an intentional browser client needs
an exact HTTPS origin.

Restart HBBS and wait at least one configured Relay-health interval:

```sh
docker compose --env-file .env -f compose.yaml restart hbbs
docker compose --env-file .env -f compose.yaml logs --tail 180 hbbs
```

Check both Upgrade paths. HTTP `101 Switching Protocols` is the expected
partial result; `curl` may end with a timeout because the upgraded connection
stays open.

```sh
for path in ws/id ws/relay; do
  curl --http1.1 --include --no-buffer --max-time 5 \
    -H 'Connection: Upgrade' \
    -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Version: 13' \
    -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    "https://rustdesk.example.com/$path"
done
```

Enable **Use WebSocket** on two clients and complete a real remote-control
session. Test WebSocket-to-WebSocket and, when required, one WebSocket client
to one native client. A `101` response by itself is not final acceptance.

## 12. Back up and record the deployment

Back up the entire persistent directory and certificate state:

```text
/opt/rustdesk-server-starry/data/
/opt/rustdesk-server-starry/.env
/opt/rustdesk-server-starry/compose.yaml
/etc/letsencrypt/
/etc/nginx/sites-available/rustdesk-starry.conf
```

At minimum, protect `data/id_ed25519`, `data/id_ed25519.pub`,
`data/db_v2.sqlite3`, `data/starry/config.yaml`, and `data/mmdb/`. Test a
restore before calling the deployment recoverable.

## 13. What is intentionally not enabled

This guide leaves connection authentication `off` because JWT issuance and
revocation require a compatible account/API service. Starry can work with
third-party API implementations; the recommended companion from the same
developer is [rustdesk-api-kessoku](https://github.com/q1ngyang/rustdesk-api-kessoku).
Read the [Kessoku Wiki](https://github.com/q1ngyang/rustdesk-api-kessoku/wiki)
and Starry's [API integration guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration)
before adding it. The dedicated joint-deployment page is still in preparation.

The optional Control Agent is not needed for RustDesk traffic. Deploy it only
when authenticated Relay visibility or managed configuration transactions are
required, beginning in read-only mode.

Continue with:

- [Docker deployment reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment)
- [Configuration reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference)
- [Client configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration)
- [Operations and verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification)
- [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting)
