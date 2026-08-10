# Operations and verification

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)

A healthy container, an open port, valid YAML, or an HTTP `101` proves only one
layer. Production acceptance requires evidence from configuration through a
real two-client desktop session. This page gives a repeatable order and clear
stop conditions.

## Evidence levels

| Level | What it proves | What it does not prove |
| --- | --- | --- |
| Static | Compose and Nginx/config syntax is valid. | A process can start or a route is reachable. |
| Process | HBBS/HBBR are running and persistent files exist. | RustDesk protocol registration or Relay flow. |
| Network | DNS, TCP/UDP, and TLS endpoints are reachable. | Correct peer identity or message exchange. |
| Protocol | Clients register and a Relay is allocated. | Desktop data remains usable under load. |
| Session | Two real clients complete the intended path. | Failover and recovery. |
| Resilience | Failure and recovery behave as designed. | Future releases; repeat after every material change. |

Keep these outcomes separate in an acceptance record: **passed**, **failed**,
and **not tested**. Never convert “not tested” into “passed”.

## 1. Capture the intended state

Before starting or changing services, record:

```sh
docker compose --env-file .env -f compose.yaml config --images
docker compose --env-file .env -f compose.yaml config > rendered-compose.review.txt
sha256sum .env compose.yaml data/starry/config.yaml > deployment-inputs.sha256
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  --format '{{json .RepoDigests}}'
```

Review the rendered Compose file before retaining it: it may include values
from `.env`. Store secrets and review evidence in protected operator storage,
not in the repository or an issue.

Also record the official upstream version, Starry patch version, architecture,
public HBBS/HBBR names, and the expected client test matrix.

## 2. Static validation

```sh
docker compose --env-file .env -f compose.yaml config --quiet
sudo nginx -t
```

For a centre/Relay topology, validate every file independently:

```sh
docker compose --env-file examples/center/.env \
  -f examples/center/compose.bootstrap.yaml config --quiet
docker compose --env-file examples/center/.env \
  -f examples/center/compose.yaml config --quiet
docker compose --env-file examples/relay/.env \
  -f examples/relay/compose.yaml config --quiet
```

Do not start the full centre file until bootstrap has generated
`id_ed25519.pub` and the optional API mount can resolve it. A successful
Compose render does not check whether the bind-mounted public-key file is
already present.

## 3. Process and persistence checks

```sh
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr

test -s data/id_ed25519
test -s data/id_ed25519.pub
test -f data/starry/config.yaml
test -f data/starry/config.example.yaml
```

Expected:

- HBBS and HBBR remain running rather than restarting;
- the HBBS health check becomes healthy;
- Starry logs show whether the external config was accepted;
- HBBR uses the same persistent public/private key pair as HBBS; and
- keys and database files survive a controlled container recreation.

Back up `id_ed25519` securely. Distribute only `id_ed25519.pub`. A new private
key changes the server identity and breaks clients configured for the old key.

## 4. Native network checks

From a network outside the server's LAN, verify TCP reachability:

```sh
nc -vz id.example.com 21115
nc -vz id.example.com 21116
nc -vz relay.example.com 21117
```

On Windows PowerShell:

```powershell
Test-NetConnection id.example.com -Port 21116
Test-NetConnection relay.example.com -Port 21117
```

UDP `21116` needs a registration/heartbeat observation or packet capture; a
generic TCP test cannot validate UDP. Confirm firewall and cloud security-group
rules in both directions and correlate the test time with HBBS logs.

Never expose `21118` or `21119` directly when Nginx is the intended public
entry point. Bind or firewall them to the reverse-proxy path.

## 5. TLS and WebSocket path checks

Check DNS and the public certificate before the Upgrade request:

```sh
openssl s_client -connect id.example.com:443 \
  -servername id.example.com -verify_return_error </dev/null

openssl s_client -connect relay.example.com:443 \
  -servername relay.example.com -verify_return_error </dev/null
```

Check the exact public paths using HTTP/1.1:

```sh
curl --http1.1 -i --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://id.example.com/ws/id

curl --http1.1 -i --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://relay.example.com/ws/relay
```

`101 Switching Protocols` proves TLS termination and the Upgrade route. Curl
does not send a RustDesk registration or Relay handshake, so the expected later
timeout is not a real-session failure and the initial `101` is not full proof.

Inspect Starry's certificate-valid Relay probes:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'websocket-status\n' | nc -w 2 127.0.0.1 21115"
```

Wait at least one configured probe interval after enabling or changing
endpoints. Every Relay intended for `wss` should be healthy; a `mixed` session
also requires that same Relay to be native-online.

## 6. Starry configuration and Geo checks

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"

docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"

docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 192.0.2.10 198.51.100.20 native\n' | nc -w 2 127.0.0.1 21115"
```

Replace the reserved addresses with the two public addresses actually observed
by HBBS. Repeat `test-geo` with `wss` and `mixed` when those paths are enabled.
Record the expected rule and Relay before running the command.

## 7. Native client acceptance

Configure two clients with the same ID Server and public key. Leave Relay
Server empty for HBBS allocation and disable WebSocket for this baseline.

Run and record:

1. both clients show ready and appear in HBBS logs;
2. direct P2P works when the network allows it;
3. a forced-Relay test reaches the intended HBBR on `21117/TCP`;
4. keyboard, pointer, clipboard, and the features relevant to your environment
   work for a sustained period; and
5. reconnect works after closing the session.

If a third-party API is deployed, separately verify API login and then a native
session after login. Evidence that `/api/status` or login succeeds does not
prove HBBS Secure TCP. Correlate the native `21116/TCP` handshake and client
session logs.

## 8. WebSocket and mixed acceptance

Enable WebSocket on clients only after the native baseline passes. Test each
matrix row independently:

| Client A | Client B | Required Relay eligibility | Expected result |
| --- | --- | --- | --- |
| WSS | WSS | WSS healthy | Session uses HBBR and remains usable. |
| WSS | Native | Native-online **and** WSS healthy | Mixed Relay session succeeds. |
| Native | WSS | Native-online **and** WSS healthy | Reverse mixed direction also succeeds. |
| Native | Native | Native-online | Existing native behaviour remains unchanged. |

For each row, capture:

- both client log excerpts with timestamps;
- HBBS registration/routing log lines;
- the selected HBBR log lines for the same Relay session; and
- at least one usable desktop/control observation.

An HTTP `101` without client `RegisterPk` and a complete HBBR session is not a
pass.

## 9. Failover and recovery

Use a maintenance window and stop only a Relay that is authorised for testing.
Do not test failover by disrupting an unrelated production node.

For each applicable transport:

1. establish the baseline on the first ordered Relay;
2. stop or firewall that Relay in a controlled, reversible way;
3. wait for the official native state and/or configured WSS failure threshold;
4. verify `test-geo` now selects the next ordered eligible Relay;
5. establish a new real session and confirm it reaches the fallback;
6. restore the first Relay;
7. wait for the success threshold; and
8. verify new sessions return to the first priority Relay.

Existing sessions may terminate during Relay loss; the acceptance target is a
correctly allocated new session unless your own availability design promises
more.

## 10. Acceptance record

Use a table like this for each release or material configuration change:

| Check | Expected | Result | Evidence/time |
| --- | --- | --- | --- |
| Compose and Nginx syntax | Valid | Not tested | |
| HBBS/HBBR stable and keys persistent | Yes | Not tested | |
| Native registration | Both clients | Not tested | |
| Native P2P | When network allows | Not tested | |
| Native Relay | Expected HBBR | Not tested | |
| Authenticated Secure TCP | When API/login is used | Not tested | |
| WSS-to-WSS | Expected HBBR | Not tested | |
| WSS-to-native | Both directions | Not tested | |
| Geo decisions | Every matrix row | Not tested | |
| Native failover/recovery | Ordered result | Not tested | |
| WSS/mixed failover/recovery | Ordered result | Not tested | |
| Backup restore rehearsal | Keys/config/state recover | Not tested | |

Sanitise peer IDs, tokens, public addresses when sharing evidence. Do not place
private keys, API credentials, or full access tokens in logs or issues.
