# Troubleshooting

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)

Diagnose one layer at a time: configuration, process, network, TLS/proxy,
registration, Relay allocation, and finally desktop data. Preserve the first
specific error and its timestamp; later reconnect attempts often add noise.

## First-response bundle

Run read-only checks before changing images, keys, firewall rules, or clients:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --since 15m hbbs hbbr
docker inspect rustdesk-starry-hbbs --format '{{json .State.Health}}'
ss -lntup | grep -E ':(21115|21116|21117|21118|21119)\b'
```

If WebSocket is involved, validate Nginx and inspect authenticated Control
Agent `GET /control/v1/status`:

```sh
sudo nginx -t
```

Record the exact image tag/digest, config hash, client versions, both clients'
public networks, whether each client has WebSocket enabled, and the same
connection attempt's timestamps. Redact secrets before sharing output.

## Container does not start or stays unhealthy

1. Read the first HBBS error, not only the health-check result.
2. Verify the bind source exists and is writable by the container.
3. Check whether another HBBS already owns `21115`, `21116`, or `21118` on the
   host network.
4. Confirm `id_ed25519` and `id_ed25519.pub` are non-empty and a matching pair.
5. Render Compose with the same `.env` actually used to start it.

The health check proves a key exists and local `21116/TCP` accepts a
connection. It cannot prove public routing or client registration.

## Starry features disappear after editing YAML

Starry rejects the complete candidate configuration when one field is unknown,
duplicated, out of range, or inconsistent. It then keeps upstream-compatible
behaviour instead of partially applying the file.

For an initial unmanaged deployment, restart HBBS and inspect the validation
log. Managed deployments should use Control Agent validation and reload:

```sh
docker restart rustdesk-starry-hbbs
docker logs --tail 200 rustdesk-starry-hbbs
```

Fix the exact reported field. Do not delete unrelated sections at random. A
schema `version: 1` file must not contain `websocket_signal`; use `version: 2`
for that section.

## Clients show “not ready” or cannot register

Work from the client towards HBBS:

- ID Server must resolve to the intended HBBS and expose `21116/TCP` plus
  `21116/UDP` for the native path.
- The client public key must exactly match `id_ed25519.pub`; remove whitespace
  introduced by copy/paste.
- API Server is not the ID Server. A successful website/API response says
  nothing about native HBBS reachability.
- Check client time and server time when TLS or token expiry is involved.
- Test from a different network to distinguish a local firewall from a server
  problem.

If only TCP is open, failure of native controlled-endpoint registration is
expected with official 1.1.16: `RegisterPk` and heartbeats use UDP, and the TCP
listener returns `NOT_SUPPORT` for that registration message. This is separate
from an authenticated controller initiation over TCP/Secure TCP. Where UDP
cannot be opened, verify WSS `/ws/id` and `/ws/relay` instead of treating the
native registration failure as a JWT denial.

Do not regenerate server keys as a connectivity experiment. That changes the
identity for every client.

## API login succeeds but control times out

Treat these as separate layers:

1. API authentication and account data;
2. native HBBS `21116/TCP` signalling, including Secure TCP when negotiated;
3. P2P or HBBR data transport.

Confirm the client ID Server and public key, then correlate HBBS handshake logs.
If the session needs Relay, separately confirm `21117/TCP` and the selected
HBBR logs. Changing an API `session_id` or reverse-proxy route cannot repair a
failed native HBBS handshake.

## `Failed to secure tcp`

Check in this order:

1. the Starry config was accepted and `secure_tcp.mode` is `auto`;
2. the client uses the correct HBBS public key;
3. `21116/TCP` reaches Starry HBBS rather than an old or duplicate process;
4. client and server versions/logs refer to the same attempt; and
5. no L4 proxy is truncating or rewriting the connection.

Secure TCP is native HBBS signalling, not WSS and not API HTTPS. Do not disable
certificate checks or change HBBR to diagnose this layer.

## Geo selects the wrong Relay

Check the precedence chain:

1. the client Relay Server field is empty;
2. HBBS observes the public IPs you tested;
3. the required MMDB is loaded and contains the expected record;
4. the specific rule precedes the catch-all;
5. `symmetric` matches the intended direction;
6. the Relay spelling equals a `relay_servers` item; and
7. the preferred Relay is eligible for the requested transport.

Remember that Relay lists are strict priority. Repeated selection of the first
healthy Relay is expected and is not failed load balancing.

## `test-geo` returns `""`

An empty result means no Relay is eligible after transport filtering. It can
result from:

- no native-online Relay for `native`;
- no certificate-valid healthy endpoint for `wss`;
- no Relay that is both native-online and WSS-healthy for `mixed`;
- a matching rule whose Relays are unavailable plus no usable later rule; or
- an empty effective Relay pool.

Compare authenticated `GET /control/v1/relays`, `GET /control/v1/status`, and
all three `POST /control/v1/allocations:simulate` transport modes. Use the
authenticated plan/apply or runtime-reload operation after changing Relay,
WebSocket, or general schema fields.

## MMDB download or lookup fails

Inspect the configured URL from the server without printing credentials. Common
causes include an HTML login/licence page, expired signed URL, TLS interception,
file smaller than `minimum_bytes`, missing MaxMind marker, or a database that
does not support the requested record type.

Starry retains the last readable database when replacement validation fails.
Confirm the file timestamp and startup/reload message before assuming fresh
data is active. `force_update: true` is a temporary recovery tool, not a normal
permanent setting.

## WSS returns 404, 400, or 502

| Response | Typical interpretation |
| --- | --- |
| `404` | Wrong hostname/path, non-exact Nginx location, or request reached a different virtual host. |
| `400` | Upgrade headers or HTTP/1.1 missing, invalid Origin, or direct backend request is malformed. |
| `502` | Nginx cannot reach `127.0.0.1:21118` for `/ws/id` or `127.0.0.1:21119` for `/ws/relay`. |

Verify exact paths, backend listeners, Nginx error logs, and `proxy_http_version
1.1`. Do not append a slash, query, or different path to a configured health
endpoint.

## WSS returns 101 but RustDesk still fails

`101` proves only HTTP Upgrade. Continue at the RustDesk protocol layer:

- confirm schema v2 or v3 and `websocket_signal.enabled: true` were accepted;
- verify the client uses `/ws/id` through the intended ID Server hostname;
- inspect registration and routing logs after Upgrade;
- confirm a certificate-valid `/ws/relay` endpoint is healthy;
- ensure a common eligible Relay exists for mixed sessions; and
- correlate both clients and HBBR for the same attempt.

If Upgrade succeeds but registration fails, changing DNS or TLS repeatedly will
not address the protocol error.

## Forwarded IP or Origin is rejected

HBBS trusts forwarded IP headers only when the direct peer is in
`trusted_proxies`. Find the source address HBBS actually sees. Add the smallest
correct proxy CIDR; never trust all addresses.

If a client sends `Origin`, it must exactly equal an item in `allowed_origins`.
An empty list rejects all Origin-bearing requests while native clients without
that header remain accepted. Scheme, host, and port are part of the origin;
paths, credentials, queries, and fragments are not valid entries.

## Native works, WSS is unhealthy

Native HBBR online state and WSS endpoint health are deliberately different.
For the Relay hostname in `websocket-status`, verify DNS, public port `443`,
certificate chain, hostname/SNI, exact `/ws/relay`, Nginx-to-`21119` reachability,
and success/failure thresholds. Ping and a normal HTTPS `200` are insufficient.

## Mixed mode has no Relay

Mixed requires the **same Relay name** to be both native-online and WSS-healthy.
Check that:

- `relay_servers` and `relay_health.endpoints[].relay` use identical
  `host:21117` values;
- the bundled unmodified HBBR reports that value online;
- its WSS endpoint is healthy in the current configuration generation; and
- the Geo rule lists that Relay.

Do not solve this by mapping native and WSS names for different physical nodes;
they must describe one HBBR service reachable by both paths.

## Session connects but is slow

First determine P2P versus Relay from client and server logs. For Relay sessions,
identify the Relay UUID/session and correlate both legs. Then measure client-to-
Relay latency, packet loss, throughput, CPU, memory, and host/network shaping.

A lower geographic distance is not guaranteed lower latency. Use real
measurements to adjust ordered rules. Restart or reload only the component
whose state actually changed, then repeat the same controlled transfer.

## Connection authentication unexpectedly denies or allows

Read `configured_mode`, `effective_mode`, `verifier_state`, key age, and metric
deltas from Control Agent status. `--must-login` can make effective mode
`enforce` even when the document says audit/off. A local claim/signature error
must not call introspection; an otherwise valid request does call a configured
introspection service and fails closed on timeout, TLS, 5xx, malformed, or
inactive responses.

Check exact `typ=at+jwt`, issuer/audience/token-use/scope, `sub == user_id`, clock sync,
explicit `kid`, Ed25519 public key rotation overlap, JWKS staleness, client
token placement, request kind, and transport. Never paste the raw token into a
ticket. Return enforce to audit through a local acknowledged reload if
legitimate clients regress; do not create a remote bypass.

A JWKS server with a shorter idle timeout can make stale pooled keep-alive
connections look like intermittent refresh failures. patch-v1.2.0 limits the
internal mTLS HTTP pool idle lifetime to 15 seconds. If failures continue,
correlate Starry refresh logs with the Kessoku idle timeout, certificate chain,
server name, and `key_age_seconds`; do not hide a persistent fault by only
increasing `max_stale_seconds`.

## Control Agent is unavailable or blocks writes

Separate TLS handshake, URI-SAN allowlist, service-JWT audience/azp/scope, and
local HBBS connectivity. A 404 for write endpoints is expected when
`write_enabled: false`. ETag/plan/idempotency conflicts are deliberate
concurrency protection, not retry-without-preconditions errors.

If an operation enters `manual_intervention_required`, stop retries. Preserve
the state/audit/recovery directories, compare exact managed config bytes and
HBBS runtime generation/digests, restore the reviewed last-known-good file, and
perform a local acknowledged reload. Follow the
[Control Agent recovery runbook](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent);
deleting state to clear the block destroys required evidence.

## Upgrade caused a regression

Stop rollout when the first reproducible protocol regression appears. Preserve
the new and old image digests, config, and logs. If safe, disable only the new
feature first; otherwise restore the previous immutable tag and configuration
according to [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback).

Do not overwrite the data directory with an empty backup or regenerate keys.
