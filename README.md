# rustdesk-server-starry

**English** | [简体中文](README.zh-CN.md)

`rustdesk-server-starry` is a lightweight overlay for the official
[`rustdesk/rustdesk-server`](https://github.com/rustdesk/rustdesk-server).
This repository does not maintain a permanently modified copy of the upstream
source. Each build fetches an exact official revision, then
[`scripts/apply_overlay.py`](scripts/apply_overlay.py) injects the Starry
modules at fixed, verified anchors.

Starry adds three feature groups:

- Select a Relay from the country, city, ASN, and ISP information of both endpoints, with strict ordered failover.
- Provide a RustDesk-client-compatible Secure TCP handshake and encrypted transport on native HBBS TCP `21116`, fixing the signalling timeout that can occur after a successful API login.
- Provide opt-in persistent WebSocket signalling on `/ws/id` for clients in restricted enterprise networks, including mixed WSS/native Relay routing and certificate-verified `/ws/relay` health filtering.

This is not a workaround that merely adds a `session_id` field. API
authentication and the HBBS signalling connection are separate layers. Starry
adds the HBBS Secure TCP compatibility required by an authenticated client.

Current overlay version: **patch-v1.1.0**. See
[`RELEASE-NOTES-patch-v1.1.0.md`](RELEASE-NOTES-patch-v1.1.0.md) for the complete
new-feature and upgrade summary.

Development status: core code and core automated acceptance are complete.
Publication is gated by the repository release workflow; a successful GitHub
Release is not evidence of a production rollout. Full TLS reverse-proxy
coverage, the 1,000-session/30-minute stress gate, real-client desktop control,
and all seven production Relay checks remain pending; see
[`PATCH-V1.1.0-WEBSOCKET-DEVELOPMENT-PLAN.md`](PATCH-V1.1.0-WEBSOCKET-DEVELOPMENT-PLAN.md).

> This is an unofficial community project and is not affiliated with RustDesk,
> MaxMind, or any MMDB mirror provider. The image does not contain a GeoLite2
> database. Operators must choose a lawful, trusted data source and comply with
> its licence.

## Compatibility and fallback guarantees

- `starry/config.yaml` is created as an empty file on the first run.
- `config.example.yaml` is created beside it and an existing file is never overwritten.
- If the configuration is empty, cannot be parsed as YAML, or fails any field validation, the entire Starry configuration is disabled and HBBS uses the official behaviour and command-line arguments.
- `secure_tcp.mode: off` preserves the official native plaintext TCP transport. `auto` enables compatible negotiation.
- Secure TCP is injected only into native TCP `21116`; it is never layered over WSS.
- Schema `version: 1` remains compatible and keeps WebSocket Signal disabled. Schema `version: 2` enables the new validated `websocket_signal` section.
- A client with WebSocket disabled keeps the native/P2P path. WebSocket is used only when that client explicitly enables it, and any session with a WSS endpoint is Relay-only.
- In `auto` mode, a valid plaintext first Protobuf frame falls back compatibly to plaintext. Once a client sends a Key Exchange, an authentication failure closes the connection and never causes an insecure downgrade.
- If no GEO rule matches, a required MMDB is unavailable, or no Relay in the matching rules is online, selection continues through the official Relay logic.

## Docker Compose deployment

Docker Compose is the preferred deployment method on a Linux server. The
repository provides complete, heavily commented
[`examples/compose.yaml`](examples/compose.yaml) and
[`examples/.env.example`](examples/.env.example) files:

```sh
cp examples/.env.example .env
cp examples/compose.yaml compose.yaml
mkdir -p data
docker compose --env-file .env up -d
```

Every GitHub Release attaches the same examples as `compose.yaml` and
`compose.env.example`:

```sh
cp compose.env.example .env
mkdir -p data
docker compose --env-file .env -f compose.yaml up -d
```

The `.env` file controls only Compose interpolation: the image tag, persistent
directory, Compose project name, container names, and restart policy. It is not
injected into HBBS or HBBR. GEO, MMDB, Secure TCP, WebSocket Signal, and Relay priority settings
always belong in the external YAML file.

The example uses `network_mode: host` and targets a Linux Docker host. It
exposes the same ports as the official server. The first start creates:

```text
data/
└── starry/
    ├── config.yaml
    └── config.example.yaml
```

Edit `data/starry/config.yaml` and restart HBBS:

```sh
docker compose restart hbbs
```

The configuration can also be reloaded through the loopback HBBS management
command:

```sh
printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115
```

Clients continue to use the same ID Server, API Server, and public key. Leave
the client's Relay Server field empty: a static client-side Relay address
bypasses dynamic allocation by HBBS.

For routine use, leave the client's **Use WebSocket** option disabled to retain
native P2P preference and performance. Enable it only on a client whose current
network cannot pass native signalling. The other endpoint may remain native;
Starry will choose a Relay that is healthy for both required transports.

## External configuration

See the complete, copy-ready
[`config/config.example.yaml`](config/config.example.yaml). Relay addresses are
always written one per line:

```yaml
version: 2

relay_servers:
  - jp-relay-1.example.com:21117
  - jp-relay-2.example.com:21117
  - us-relay-1.example.com:21117

secure_tcp:
  mode: auto
  handshake_timeout_ms: 18000
  idle_timeout_ms: 30000
  max_frame_bytes: 65536

websocket_signal:
  enabled: true
  registration_timeout_ms: 10000
  keepalive_interval_ms: 12000
  idle_timeout_ms: 45000
  max_frame_bytes: 65536
  outbound_queue_capacity: 64
  max_sessions: 10000
  max_sessions_per_effective_ip: 512
  registration_rate_per_minute: 300
  trusted_proxies: [127.0.0.1/32, "::1/128"]
  allowed_origins: []
  relay_health:
    interval_seconds: 60
    timeout_ms: 5000
    success_threshold: 1
    failure_threshold: 2
    endpoints:
      - relay: jp-relay-1.example.com:21117
        url: wss://jp-relay-1.example.com/ws/relay
      - relay: jp-relay-2.example.com:21117
        url: wss://jp-relay-2.example.com/ws/relay
      - relay: us-relay-1.example.com:21117
        url: wss://us-relay-1.example.com/ws/relay

mmdb:
  update_interval_hours: 168
  update_on_start: true
  force_update: false
  download_timeout_seconds: 600
  minimum_bytes: 65536
  country:
    path: mmdb/GeoLite2-Country.mmdb
    url: https://example.com/GeoLite2-Country.mmdb
  city:
    path: mmdb/GeoLite2-City.mmdb
    url: https://example.com/GeoLite2-City.mmdb
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: https://example.com/GeoLite2-ASN.mmdb

geo:
  enabled: true
  rules:
    - name: East Asia
      symmetric: true
      match:
        client_a: "CN/JP/KR/TW"
        client_b: "*"
      relays:
        - jp-relay-1.example.com:21117
        - jp-relay-2.example.com:21117
```

Relative MMDB paths are resolved from the HBBS working directory. The Compose
working directory is `/root`, so the example stores the databases in
`./data/mmdb/` on the host.

An MMDB download is written to a temporary file, checked for minimum size, the
MaxMind marker, and database readability, then used to replace the old file.
The last usable database is retained when download or validation fails.
`force_update: true` downloads again on every update cycle.
`update_interval_hours: 0` disables periodic updates.

## Opt-in WebSocket Signal

`websocket_signal.enabled: true` upgrades the existing HBBS WebSocket listener
into a persistent, identity-validated signalling path. It does not turn
WebSocket on for any client. Clients choose the transport independently:

- WebSocket off: native signalling, P2P preferred, native Relay when needed.
- WebSocket on: WSS signalling and WSS Relay; P2P is intentionally not offered.
- One endpoint WSS and one native: both use the same HBBR node and Relay UUID,
  with WSS on one side and native `21117` on the other.

Each `relay_servers` entry must have exactly one `relay_health.endpoints` item.
WSS probes perform normal DNS, TCP, TLS certificate/hostname validation, and an
exact `/ws/relay` WebSocket Upgrade. An HTTPS 200 response, ping, or an insecure
TLS connection is not considered healthy. A mixed session additionally
requires the same node to be present in HBBS's native online Relay list.

Forwarded IP headers are accepted only when the direct TCP peer belongs to
`trusted_proxies`. The proxy connection's unique source port is retained only
as an internal reply correlation token and never treated as a client NAT port.
A missing Origin is accepted for native RustDesk clients. If an Origin header
is present, it must exactly match one `allowed_origins` entry.

Terminate TLS at a trusted reverse proxy and forward the two native WebSocket
paths without rewriting them:

```nginx
location = /ws/id {
    proxy_pass http://127.0.0.1:21118;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 120s;
}

location = /ws/relay {
    proxy_pass http://127.0.0.1:21119;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 120s;
}
```

Restrict direct access to port `21118` to the configured proxy network in
production. Review runtime status through loopback-only management port 21115:

```sh
printf 'websocket-status\n' | nc -w 2 127.0.0.1 21115
```

The output contains aggregate session/counter data and per-Relay native/WSS
health, but no full Peer ID, public key, token, or raw client-IP list.

## GEO expressions

The expression operators are:

- `/`: OR.
- `+`: AND, with higher precedence than `/`.
- `(...)`: explicit grouping with arbitrary nesting.
- `*`: match any location.

For example:

```yaml
match:
  client_a: "(city:City A+isp:Carrier B)/city:City C"
  client_b: "*"
```

This means `(City A AND Carrier B) OR City C`.

```yaml
match:
  client_a: "((city:City A+isp:Carrier B)/(city:City C+isp:Carrier D))+country:CN"
  client_b: "*"
```

This means `((City A AND Carrier B) OR (City C AND Carrier D)) AND country CN`.

A bare two-letter ASCII token is treated as an ISO 3166-1 country code, so
multiple countries can be combined as:

```yaml
client_a: "CN/JP/KR/TW"
```

Supported explicit fields:

| Field | Database | Match behaviour |
| --- | --- | --- |
| `continent` | Country/City | Two-letter continent code, case-insensitive |
| `country` | Country/City | Two-letter country code, case-insensitive |
| `subdivision` / `region` | City | Subdivision code or name |
| `city` | City | Any localised city name present in the database |
| `geoname` / `city_id` | City | Non-zero GeoNames ID |
| `asn` | ASN | Non-zero ASN; `AS4134` is accepted |
| `isp` / `asn_org` | ASN | Case-insensitive substring match on organisation name |

Wrap a value in single or double quotes if the value itself contains `/`,
`+`, or parentheses:

```yaml
client_a: "city:\"A/B\"+isp:'Carrier X+Y'"
```

Each rule matches `client_a` and `client_b` separately. `symmetric: true` is
the default and also evaluates the rule after swapping both endpoints. Set it
to `false` for a direction-sensitive rule.

## Relay ordering and failover

Rules are evaluated from top to bottom. Relays inside a rule are also selected
strictly from top to bottom:

```yaml
relays:
  - relay-priority-1.example.com:21117
  - relay-priority-2.example.com:21117
  - relay-priority-3.example.com:21117
```

The first Relay remains preferred whenever it is present in the official HBBS
online list. Starry does not round-robin inside a matching rule. The next Relay
is selected only after the preceding Relay fails the official health check. If
all Relays in a matching rule are offline, Starry checks subsequent rules and
finally falls back to official selection.

Here, a failure means reachability from HBBS to the Relay. The RustDesk OSS
protocol does not report each client's Relay latency, packet loss, or connection
failure back to HBBS. Starry therefore does not present a ping from the centre
server to HBBR as if it represented client-side path quality.

## Inspecting assignable Relays and testing two IPs

HBBS exposes management commands on local `21115/TCP`. Docker and local
deployments use exactly the same commands and selection logic; only the access
method differs. With Compose, run the command inside the HBBS container. A
Linux/DEB or Windows binary is queried directly on its host.

Management commands are accepted only from a loopback address in the network
namespace that contains HBBS. Do not add a public proxy or remote forwarding
endpoint for these commands.

### List the Relays HBBS can currently assign

`relay_servers` in the configuration file is the complete candidate pool. For
Compose, inspect `data/starry/config.yaml`. The DEB uses
`/etc/rustdesk-server-starry/config.yaml`. A directly launched binary uses the
path supplied through `--starry-config`, which defaults to
`starry/config.yaml` under its current working directory.

The `relay-servers` command, abbreviated as `rs`, takes no argument when used
as a query. It prints the Relays currently participating in HBBS allocation,
one per line:

```text
jp-relay-1.example.com:21117
jp-relay-2.example.com:21117
```

With multiple Relays, official HBBS refreshes reachability approximately every
three seconds. Wait a few seconds after startup or reload before querying. The
output order is not the Starry priority order; priority always comes from the
`relays` list in the matching rule. This is also an HBBS allocation view, not a
measurement of end-to-end quality from either client.

Compose deployment:

If `STARRY_HBBS_CONTAINER_NAME` was changed in `.env`, replace
`rustdesk-starry-hbbs` below with that value.

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"
```

Linux binary or DEB deployment:

```sh
printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115
```

### Provide two IPs and preview the selected Relay

`test-geo`, abbreviated as `tg`, accepts two literal IP addresses and an
optional transport requirement. It invokes the same selection function used by
live signalling with the current MMDB readers, rules, and Relay health state:

```text
test-geo <IP_A> <IP_B> [native|wss|mixed]
```

Omitting the final argument preserves the patch-v1.0.0 `native` behaviour.
`wss` requires a healthy `/ws/relay`; `mixed` requires both WSS health and the
same node's native online status.

The first address maps to `client_a` and the second to `client_b`. A rule with
the default `symmetric: true` is also evaluated after swapping both endpoints.
For a direction-sensitive rule, test both `A B` and `B A`. Use the public
egress addresses that HBBS actually observes, not client-side
`192.168.x.x` or `10.x.x.x` addresses. If both clients share the same public
NAT, supply that same public IP twice.

Compose example; replace the addresses with the clients' real public IPs:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 1.1.1.1 8.8.8.8 mixed\n' | nc -w 2 127.0.0.1 21115"
```

Linux binary or DEB example:

```sh
printf 'test-geo 1.1.1.1 8.8.8.8 mixed\n' | nc -w 2 127.0.0.1 21115
```

For a local Windows binary, first define this PowerShell helper:

```powershell
function Invoke-StarryHbbsCommand {
    param([Parameter(Mandatory)][string]$Command)

    $client = [System.Net.Sockets.TcpClient]::new()
    $result = [System.IO.MemoryStream]::new()
    try {
        $client.Connect('127.0.0.1', 21115)
        $stream = $client.GetStream()
        $stream.ReadTimeout = 2000
        $request = [System.Text.Encoding]::UTF8.GetBytes("$Command`n")
        $stream.Write($request, 0, $request.Length)
        $buffer = [byte[]]::new(4096)
        while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $result.Write($buffer, 0, $count)
        }
        [System.Text.Encoding]::UTF8.GetString($result.ToArray())
    }
    finally {
        $result.Dispose()
        $client.Dispose()
    }
}

Invoke-StarryHbbsCommand 'relay-servers'
Invoke-StarryHbbsCommand 'test-geo 1.1.1.1 8.8.8.8'
```

A normal result is quoted:

```text
"jp-relay-1.example.com:21117"
```

- `""` means that HBBS currently has no Relay to assign.
- No output at all usually means that the command syntax or an IP address could not be parsed.
- When a Starry rule matches, the result remains the first online Relay in that rule until reachability changes.
- If a matching rule has no online Relay, later rules are checked. Only after no rule can provide a Relay does the result come from official fallback. Repeated tests can return different nodes when official fallback round-robin has multiple Relays.

This command previews the allocation decision at that instant. It does not
create a connection, force either client to use a Relay, or prove client-to-Relay
reachability. A real session may still establish P2P. HBBS sends this selection
only after a session enters the Relay path. A later configuration reload, MMDB
update, or health-state change can also change the live result.

## Secure TCP state machine

With `secure_tcp.mode: auto`, native TCP `21116` follows this state machine:

```text
HBBS sends a Curve25519 public key signed by the server Ed25519 identity
  → the client verifies the signature
  → the client sends its Curve25519 public key and sealed symmetric key
  → both sides use independent send/receive nonce sequences for Secretbox frames
```

Starry reuses the official HBBS identity key and introduces no second shared
key. HBBS generates the required identity by default. If the server identity
key is explicitly disabled, `auto` cannot offer Secure TCP and retains the
official plaintext transport.

Compatibility tests cover the signed public-key format, two-element Key
Exchange, sealed-key authentication, independent send and receive counters,
ciphertext authentication failure, a real TCP encrypted round trip, and valid
plaintext-first-frame fallback.

## Local release artifacts

Each Starry Release provides only these platforms and architectures:

| Platform | Architecture | Artifacts |
| --- | --- | --- |
| Docker | `linux/amd64`, `linux/arm64` | One multi-architecture GHCR image |
| Linux | `amd64`, `arm64` | Separate `hbbs`, `hbbr`, and `rustdesk-utils` binaries plus a tar.gz |
| Debian/Ubuntu | `amd64`, `arm64` | Three independently installable DEB packages |
| Windows | `amd64` | Three separate `.exe` files plus a zip |

DEB package names:

```text
rustdesk-server-starry-hbbs
rustdesk-server-starry-hbbr
rustdesk-server-starry-utils
```

The HBBS DEB installs an empty configuration and its example:

```text
/etc/rustdesk-server-starry/config.yaml
/etc/rustdesk-server-starry/config.example.yaml
```

The services are managed by systemd:

```sh
sudo systemctl status rustdesk-server-starry-hbbs
sudo systemctl status rustdesk-server-starry-hbbr
```

Windows can run a standalone executable from the release page:

```powershell
& .\hbbs-<release>-windows-amd64.exe --starry-config=.\starry\config.yaml
```

The first run creates `starry\config.yaml` and
`starry\config.example.yaml` under the current directory.

## Automatic upstream release tracking

The version format is:

```text
<official-version>-patch-vX.Y.Z
```

- `X` is the Starry major version and changes only for a major feature or compatibility break.
- `Y` is incremented for routine feature releases.
- `Z` is incremented only for an urgent fix to the current patch release.

The scheduled release flow is:

```text
discover the latest official formal Release
  → fetch the exact upstream source and submodules
  → verify and apply the overlay twice
  → lock dependencies and run all tests
  → build amd64, arm64, Windows, and independent DEBs
  → start and smoke-test both container architectures
  → publish the GitHub Release and GHCR image only when everything succeeds
```

A functional or architecture failure stops the release and creates a GitHub
Issue. If logs classify the failure as GitHub Runner resource exhaustion,
communication loss, or timeout, the controller retries at 10, 30, and 90
minutes. It stops and notifies only after all three retries fail.

The first Starry release is excluded from automatic retries and is not
published until the README, Release content, and image preview are approved.
After that release succeeds, setting the repository variable
`STARRY_RELEASE_ENABLED=true` enables unattended publication for later formal
upstream releases.

## Overlay development

This repository stores the overlay modules, injection script, configuration
template, packaging files, and workflows. To validate an official checkout:

```sh
python3 scripts/apply_overlay.py /path/to/clean/rustdesk-server
python3 scripts/apply_overlay.py /path/to/clean/rustdesk-server
git -C /path/to/clean/rustdesk-server diff --check
```

The script must be idempotent. A missing or duplicated fixed anchor causes an
immediate failure so an upstream structural change stops publication instead
of producing an incomplete hard fork.

## Licence

The upstream RustDesk Server and this project remain under `AGPL-3.0`.
Published artifacts are built from the corresponding official revision plus
this repository's overlay.
