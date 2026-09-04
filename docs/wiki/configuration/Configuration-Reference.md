# Configuration reference

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)

Starry reads `starry/config.yaml` relative to the HBBS data directory. In the
container this is `/root/starry/config.yaml`, because `/root` is the persistent
data mount. On first start HBBS also creates
`starry/config.example.yaml` as a local reference.

The parser rejects unknown fields, duplicate list values, invalid ranges, and
cross-references to Relays that are not declared in `relay_servers`. On first
load, a missing, empty, or invalid file does **not** partially enable Starry:
HBBS logs the error and keeps upstream-compatible behaviour. After a valid
generation is active, a rejected reload retains that complete last-known-good
generation. Treat both outcomes as safety properties, not as a reason to
ignore logs or activation acknowledgements.

## Document version and feature gates

| Field | Required | Accepted values | Meaning |
| --- | --- | --- | --- |
| `version` | Yes | `1`, `2`, `3`, `4`, `5` | Configuration schema. v1.3.2 keeps patch-v1.3.1 schema `5` byte-identical. |

Schema `1` supports Relay, Secure TCP, MMDB, and Geo settings and rejects
`websocket_signal` and `connection_auth`. Schema `2` adds optional WebSocket
Signal and rejects `connection_auth`. Schema `3` adds connection
authentication and rejects `relay_quality`. Schema `4` adds opt-in Akari Relay
quality selection and FastCompat Relay authorization. Schema `5` adds
FastMediaV1 policy and a declared Relay UDP endpoint without changing schema-4
or Relay Quality v1 semantics.
Unknown top-level and nested keys are rejected so that
misspellings cannot silently change a deployment.

## `relay_servers`

```yaml
relay_servers:
  - relay-asia-1.example.com:21117
  - relay-us-1.example.com:21117
```

This is the complete Relay allocation pool known to Starry HBBS. Values are
trimmed and must be non-empty and unique, case-insensitively.

- Every Relay referenced by a Geo rule must appear here.
- When WebSocket Signal is enabled, `relay_health.endpoints` must cover this
  list exactly, one WSS endpoint per Relay.
- A host and port identify the native HBBR destination. This is not an HTTP URL.
- Keep a RustDesk client's **Relay Server** field empty if HBBS should make the
  selection. A client-side Relay value overrides server allocation.

## `secure_tcp`

```yaml
secure_tcp:
  mode: auto
  handshake_timeout_ms: 18000
  idle_timeout_ms: 30000
  max_frame_bytes: 65536
```

| Field | Default | Valid range or values | Notes |
| --- | ---: | --- | --- |
| `mode` | `off` | `off`, `auto` | `auto` negotiates the client-compatible encrypted native signalling transport while still accepting a valid plaintext first frame. |
| `handshake_timeout_ms` | `18000` | `1000..120000` | Maximum negotiation time. |
| `idle_timeout_ms` | `30000` | `1000..600000` | Idle read timeout for the negotiated transport. |
| `max_frame_bytes` | `65536` | `4096..16777216` | Maximum accepted secure frame size. Raise only with a measured need. |

Secure TCP applies to native HBBS signalling on `21116/TCP`. It does not
encrypt or proxy HBBR by itself, and it is distinct from HTTPS used by an API.

## `mmdb`

```yaml
mmdb:
  update_interval_hours: 168
  update_on_start: true
  force_update: false
  download_timeout_seconds: 600
  minimum_bytes: 65536
  country:
    path: mmdb/GeoLite2-Country.mmdb
    url: https://downloads.example.com/GeoLite2-Country.mmdb
  city:
    path: mmdb/GeoLite2-City.mmdb
    url: https://downloads.example.com/GeoLite2-City.mmdb
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: https://downloads.example.com/GeoLite2-ASN.mmdb
```

| Field | Default | Valid range | Meaning |
| --- | ---: | --- | --- |
| `update_interval_hours` | `168` | `0..8760` | Periodic refresh interval. `0` disables periodic refresh. |
| `update_on_start` | `true` | Boolean | Checks configured download URLs during startup. |
| `force_update` | `false` | Boolean | Downloads even when the current file is still within the interval. Use temporarily, then turn it off. |
| `download_timeout_seconds` | `600` | `1..3600` | Per-download timeout. |
| `minimum_bytes` | `65536` | `1024..1073741824` | Rejects implausibly small downloads. This is not a licence or authenticity check. |

Each of `country`, `city`, and `asn` has:

| Field | Default | Rule |
| --- | --- | --- |
| `path` | `mmdb/GeoLite2-Country.mmdb`, `mmdb/GeoLite2-City.mmdb`, or `mmdb/GeoLite2-ASN.mmdb` | Must be a relative `mmdb/*.mmdb` path without traversal. Symbolic-link components are rejected before replacement. |
| `url` | empty | Optional `https://` download URL. Empty means local-file management. Redirects are rejected. Use a source you are licensed to use. |

Downloads are written to a temporary file, checked for the minimum size,
MaxMind marker, and reader compatibility, then atomically replace the target.
Responses larger than 1 GiB are rejected before replacement. On failure the
previous readable file remains in place. The image does not contain GeoLite2
data and does not provide a database licence.

Choose only the databases required by your rules:

| Rule field | Required database |
| --- | --- |
| `continent`, `country`, or bare country code | Country **or** City |
| `subdivision`, `region`, `city`, `geoname`, `city_id` | City |
| `asn`, `isp`, `asn_org` | ASN |

## `geo`

```yaml
geo:
  enabled: true
  rules:
    - name: Asia preference
      symmetric: true
      match:
        client_a: CN/JP/KR
        client_b: "*"
      relays:
        - relay-asia-1.example.com:21117
        - relay-asia-2.example.com:21117
```

| Field | Default | Rule |
| --- | ---: | --- |
| `enabled` | `false` | Enabling requires at least one rule and one `relay_servers` entry. |
| `rules[].name` | None | Required, non-empty, and unique. |
| `rules[].symmetric` | `true` | When true, Starry also tries A/B with the client positions exchanged. |
| `rules[].match.client_a` | `*` | Expression for the first observed public client address. |
| `rules[].match.client_b` | `*` | Expression for the second observed public client address. |
| `rules[].relays` | None | Required, ordered, unique Relay list; every item must exist in `relay_servers`. |

Rules are evaluated from top to bottom. Inside a matching rule, Relays are
strict priority: the first currently eligible Relay wins. Continue with
[GEO Rules: Basics](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics)
and [GEO Rules: Advanced](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Advanced).

## `websocket_signal`

This section requires `version: 2`, `3`, `4`, or `5` and is opt-in.

```yaml
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
  trusted_proxies:
    - 127.0.0.1/32
    - ::1/128
  allowed_origins: []
  relay_health:
    interval_seconds: 60
    timeout_ms: 5000
    success_threshold: 1
    failure_threshold: 2
    endpoints:
      - relay: relay-asia-1.example.com:21117
        url: wss://relay-asia-1.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry
        fast_media_udp_port: 21119
```

### Session and resource limits

| Field | Default | Valid range |
| --- | ---: | --- |
| `enabled` | `false` | Boolean |
| `registration_timeout_ms` | `10000` | `1000..120000` |
| `keepalive_interval_ms` | `12000` | `1000..300000`, and lower than `idle_timeout_ms` |
| `idle_timeout_ms` | `45000` | `2000..600000` |
| `max_frame_bytes` | `65536` | `4096..16777216` |
| `outbound_queue_capacity` | `64` | `1..4096` |
| `max_sessions` | `10000` | `1..1000000` |
| `max_sessions_per_effective_ip` | `512` | `1..max_sessions` |
| `registration_rate_per_minute` | `300` | `1..100000` |

These limits protect HBBS resources; they are not a substitute for firewall,
reverse-proxy, and host monitoring controls.

### Proxy identity and Origin

`trusted_proxies` contains unique CIDR ranges whose forwarded client-IP header
may be trusted. The defaults trust only `127.0.0.1/32` and `::1/128`, suitable
when Nginx shares the host network with HBBS. Add a Docker bridge or external
proxy subnet only after confirming the real source address seen by HBBS. Never
use `0.0.0.0/0` merely to make a header work.

`allowed_origins` is an optional list of exact `http://` or `https://` origins,
with no path, credentials, query, or fragment. Native RustDesk clients that do
not send an `Origin` remain accepted. Any client that sends one must match an
item exactly; an empty list rejects every Origin-bearing request.

### `relay_health`

| Field | Default | Valid range or rule |
| --- | ---: | --- |
| `interval_seconds` | `60` | `5..3600` |
| `timeout_ms` | `5000` | `500..120000` |
| `success_threshold` | `1` | `1..100` consecutive successes |
| `failure_threshold` | `2` | `1..100` consecutive failures |
| `endpoints[].relay` | None | Required, unique, and equal to one `relay_servers` item. |
| `endpoints[].url` | None | Required unique URL: `wss://` plus a DNS hostname and exact `/ws/relay` (legacy health only) or `/ws/telemetry` path; no credentials, query, or fragment. |
| `endpoints[].telemetry_secret_file` | None | Absolute secret-file path; required only for `/ws/telemetry` and forbidden for `/ws/relay`. The file value is never serialized. |
| `endpoints[].fast_media_udp_port` | None | Schema v5 only, `1..65535`; permitted only with authenticated `/ws/telemetry`. It declares an endpoint but does not prove listener health. |

When `enabled: true`, endpoint Relay names must cover `relay_servers` exactly.
The health probe verifies the WSS/TLS path used for allocation; it does not
replace a two-client remote-control test. See
[Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS).

## `connection_auth`

This section requires `version: 3`, `4`, or `5`. It gates controller-side
`PunchHoleRequest` and direct `RequestRelay` on native TCP, Secure TCP, and WSS.
UDP initiation remains unsupported and does not allocate.

```yaml
connection_auth:
  mode: audit
  issuer: https://kessoku.example
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  max_token_bytes: 8192
  clock_skew_seconds: 30
  jwks:
    file: /var/lib/starry-auth/jwks.json
    url: https://kessoku.example/api/internal/v1/auth/jwks
    refresh_interval_seconds: 300
    max_stale_seconds: 3600
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
  introspection:
    required: true
    url: https://kessoku.example/api/internal/v1/auth/introspect
    timeout_ms: 1000
    positive_cache_seconds: 10
    negative_cache_seconds: 1
    max_cache_entries: 100000
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
```

| Field | Default | Rule |
| --- | --- | --- |
| `mode` | `off` | `off`, `audit`, or `enforce`. `audit` records decisions but proceeds. A deployment `--must-login` floor forces effective enforce. |
| `issuer` | empty | Required HTTPS issuer in audit/enforce; must exactly match `iss`. |
| `audience` | empty | Required in audit/enforce; must be present in `aud`. |
| `token_use` | `access` | Exact required `token_use` claim. |
| `required_scope` | `connect:initiate` | One complete scope value, never a substring. |
| `max_token_bytes` | `8192` | `128..8192`; checked before JWT parsing. |
| `clock_skew_seconds` | `30` | `0..300` for `iat`, `nbf`, and `exp`. |
| `jwks.file` | empty | Local public Ed25519 JWKS and durable cache path. Enforce requires a non-empty initial file. |
| `jwks.url` | empty | Optional internal HTTPS refresh URL; when present, CA/cert/key/server-name are mandatory and the URL host must equal `server_name`. |
| `jwks.refresh_interval_seconds` | `300` | `30..86400`. |
| `jwks.max_stale_seconds` | `3600` | `30..604800`; verification fails closed after this age. |
| `jwks.ca_file` / `cert_file` / `key_file` / `server_name` | empty | TLS 1.3-only mTLS trust and client identity for JWKS refresh; system roots are disabled. |
| `introspection.required` | `false` | If true, omission of the client is invalid. A configured client always fails closed on request errors regardless of this flag. |
| `introspection.url` | empty | TLS 1.3 HTTPS only. When present, all CA/cert/key/server-name fields are mandatory, system roots are disabled, and the URL host must equal `server_name`. |
| `introspection.timeout_ms` | `1000` | `100..10000`; one retry is limited to server errors. |
| `positive_cache_seconds` | `10` | `1..60`, capped by token expiry. |
| `negative_cache_seconds` | `1` | `0..1`. |
| `max_cache_entries` | `100000` | `1..1000000`; oldest entries are evicted deterministically. |

Only EdDSA/Ed25519 public JWKs with a unique explicit `kid` are accepted.
Private/symmetric/duplicate key material is rejected. Raw tokens are not used
as cache keys or status labels. See
[Connection Authentication](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Connection-Authentication)
before enabling audit or enforce.

## `relay_quality`

This frozen Akari extension requires `version: 4` or `5` and is disabled by default.
Official clients never opt in and keep the ordinary single-Relay allocation.

```yaml
relay_quality:
  enabled: true
  strategy: adaptive
  legacy_fallback_relays: []
  max_candidates: 3
  primary_probe_samples: 3
  primary_accept_score: 8000
  primary_max_loss_basis_points: 500
  p2p_probe_grace_ms: 300
  probe_samples: 5
  probe_interval_ms: 50
  probe_timeout_ms: 1000
  report_timeout_ms: 15000
  max_telemetry_age_seconds: 180
  allocation_ttl_seconds: 30
  cache_ttl_seconds: 300
  max_allocations: 10000
  hysteresis_basis_points: 500
  missing_report_penalty_basis_points: 1000
  rtt_bad_ms: 300
  jitter_bad_ms: 100
  weights: {rtt: 4000, jitter: 2000, loss: 2500, load: 1500}
```

| Field | Default | Valid range or rule |
| --- | ---: | --- |
| `enabled` | `false` | Enabling requires at least two non-legacy quality Relays, complete health endpoint coverage, and `max_candidates >= 2`. |
| `strategy` | `adaptive` | `adaptive` probes the GEO primary first and expands only when needed; `eager` probes every candidate immediately. |
| `legacy_fallback_relays` | `[]` | Unique subset of `relay_servers`; explicit ordinary fallback only, never a quality candidate. |
| `max_candidates` | `3` | `1..5` while disabled, `2..5` while enabled. |
| `primary_probe_samples` | `3` | `1..20` and no greater than `probe_samples`; sequential samples for the GEO primary. |
| `primary_accept_score` | `8000` | `1..10000`; interpreted only by HBBS. |
| `primary_max_loss_basis_points` | `500` | `0..10000`; either available endpoint exceeding it triggers expansion. |
| `p2p_probe_grace_ms` | `300` | `0..5000`; lets a successful P2P path cancel before active probing. |
| `probe_samples` | `5` | `3..20` attempts per Relay and endpoint. |
| `probe_interval_ms` | `50` | `20..2000`. |
| `probe_timeout_ms` | `1000` | `100..5000`; hard timeout for one sample. Candidates run concurrently; samples within one candidate run in order. |
| `report_timeout_ms` | `15000` | `1000..60000`; server-enforced total deadline. Adaptive must fit two primary windows, one concurrent expansion window, and 1000 ms signalling margin; eager must fit two full windows. |
| `max_telemetry_age_seconds` | `180` | `5..3600` and at least health interval plus timeout; older load excludes the quality candidate. |
| `allocation_ttl_seconds` | `30` | `5..300` and greater than `report_timeout_ms`; cleanup only, never report validity. |
| `cache_ttl_seconds` | `300` | `30..86400` for symmetric `/24` or `/56` network-pair choices. |
| `max_allocations` | `10000` | `100..1000000`; hard cap for each pending-allocation, decision, and prefix-cache map; oldest entries are evicted first. |
| `hysteresis_basis_points` | `500` | `0..5000`; keep the cached Relay unless the new score improves by more than this margin. |
| `missing_report_penalty_basis_points` | `1000` | `0..10000` per missing endpoint measurement. |
| `rtt_bad_ms` | `300` | `10..10000`; RTT normalization ceiling. |
| `jitter_bad_ms` | `100` | `1..5000`; jitter normalization ceiling. |
| `weights` | `4000/2000/2500/1500` | RTT/jitter/loss/load values must each be positive and sum to `10000`. |

Every non-legacy quality Relay must have one unique, authenticated
`/ws/telemetry` endpoint and an absolute `telemetry_secret_file`; URLs must
also be unique. `/ws/relay` is accepted only for explicit legacy health/fallback.
When
WebSocket Signal is enabled its existing exact all-Relay coverage rule still
applies. Candidate probes and reports remain inside HBBS signalling. Kessoku may manage
the configuration and read Control API counters, but neither Akari nor HBBR
connects to the Control Agent. Set `STARRY_RELAY_MAX_SESSIONS` on every HBBR;
it is an enforced admission limit. `TOTAL_BANDWIDTH` remains its capacity in
Mbit/s. HBBR receives the same secret through
`STARRY_RELAY_TELEMETRY_SECRET_FILE`; internal mTLS is preferred, while the
secret-file HMAC protects deployments whose reverse proxy terminates TLS. HBBS
trusts only the signed telemetry it fetched itself for load scoring. Those
certificate-verified probes run while
Relay quality is enabled even when `websocket_signal.enabled` is false. HBBR
must explicitly advertise probe/load protocol v1; a version string is never
treated as capability. Missing, incomplete, or stale telemetry excludes the
Relay from quality offers but leaves it available for explicitly configured
ordinary fallback.

The public `/ws/relay` handshake and `RelayProbeResponse` never contain detailed
load. HBBR probe limits default to 120 per transport-source IP and 10,000 global
per minute, configurable with `STARRY_RELAY_PROBE_PER_IP_PER_MINUTE` and
`STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE`. `STARRY_RELAY_DRAINING=true` or an
existing `STARRY_RELAY_DRAINING_FILE` refuses new pairs while existing sessions
continue. See [Relay Telemetry v1](../../reference/RELAY-TELEMETRY-v1.md).

## `fast_mode.relay`

Schema v4 supports only FastCompat. Schema v5 retains that reliable path and
adds independent FastMediaV1 Relay UDP policy. Both switches default to false.
The client never chooses the signed Relay, and every UDP failure leaves the
ordinary HBBR session available.

```yaml
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
    relay_max_datagram: 1200
```

| Field | Default | Valid range or rule |
| --- | ---: | --- |
| `fast_compat_enabled` | `false` | Requires connection authentication in `audit` or `enforce` and either `secure_tcp.mode: auto` or enabled WebSocket signalling. Relay Quality is authoritative when it decides; otherwise HBBS signs only its ordinary GEO/failover selection. |
| `fast_media_v1_enabled` | `false` | Schema v5 only. Bootstrap requires authenticated fresh telemetry schema 2 and `fast_media_relay_udp = 1`; active renewal additionally requires schema 3 and `fast_media_relay_renewal = 1`. The declared UDP port must match a healthy listener. |
| `authorization_ttl_seconds` | `90` | `30..300`; checked even while the feature is disabled. Retries do not extend expiry. |
| `max_bitrate_kbps` | `50000` | `1000..200000`; signed encoded-source ceiling. HBBR wire allowance is at most `ceil(source × 1.45)`. |
| `relay_max_datagram` | `1200` | Schema v5 only, `608..1400`; complete UDP payload including the 32-byte AKR1 header. |

HBBS signs only after authentication returns exact `allow` and the ordinary
final Relay is fixed. A Relay Quality decision, when present, must exactly
match that Relay; otherwise the server-selected GEO/failover Relay is used.
FastMedia produces role 1 controller and role 2 target grants and begins only
after both bind at HBBR. FastCompat-only six-field grants remain compatible.
Any missing prerequisite produces no FastMedia grant and preserves the
ordinary Relay flow. Official clients ignore tag 64.

patch-v1.3.2 adds no YAML key. These HBBR environment limits govern renewable
allocations and are included in authenticated telemetry v3:

| Environment variable | Default | Accepted range | Meaning |
| --- | ---: | ---: | --- |
| `STARRY_RELAY_FAST_MEDIA_MAX_SESSION_TTL_SECONDS` | `43200` | `600..86400` | Absolute lifetime from allocation creation; renewal cannot make a session immortal. |
| `STARRY_RELAY_FAST_MEDIA_RENEWAL_TRANSITION_SECONDS` | `15` | `5..60` | Maximum bounded period while role grant sequences differ by one. |
| `STARRY_RELAY_FAST_MEDIA_POST_EXPIRY_RECOVERY_SECONDS` | `30` | `10..120` | Retention after role-grant expiry for a valid renewed rebind; expired grants never authorize media. |
| `STARRY_RELAY_FAST_MEDIA_PER_IP_BYTES_PER_SECOND` | `33554432` | `65536..1073741824` | Aggregate normalized-source-IP wire-rate admission budget. Same-NAT roles add together. |
| `STARRY_RELAY_FAST_MEDIA_GLOBAL_BYTES_PER_SECOND` | `536870912` | `1048576..8589934592` | Global reserved wire-rate admission budget. |

Only HBBS-authenticated telemetry v3 can turn on the typed renewal capability.
The public probe, a version string, and Control API input cannot do so. See
[FastMedia active-session renewal v1](../../reference/FAST-MEDIA-RENEWAL-v1.md)
and [Relay telemetry v3](../../reference/RELAY-TELEMETRY-v3.md).

If WSS terminates at a reverse proxy, deny direct public access to HBBS's
plaintext WebSocket listener. See the
[Fast Relay Authorization v1 contract](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/reference/FAST-RELAY-AUTHORIZATION-v1.md)
and [FastMedia Relay UDP v1](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/reference/FAST-MEDIA-RELAY-UDP-v1.md)
for wire, replay, retry, resource, and privacy requirements.

## Reload behaviour

For initial commissioning without the Control Agent, restart HBBS after editing:

```sh
docker restart rustdesk-starry-hbbs
```

Subsequent managed changes should use the authenticated versioned Control API
plan/apply or `POST /control/v1/runtime:reload` flow. Activation is atomic at
the active-generation level. A complete valid candidate is
prepared by each required subsystem and becomes active with a new generation
only after every acknowledgement succeeds. An empty/invalid/rejected reload
retains the prior last-known-good generation, digests, Relay/auth state, and
reports `last_error`. If no valid generation has ever loaded, HBBS keeps
upstream-compatible behaviour. Correct or restore the disk file and require a
successful activation acknowledgement; process survival alone is not success.

## Ready-made profiles

- [`config.single-host.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.single-host.yaml): complete single-host commissioning profile, with Geo and WebSocket disabled until their prerequisites are ready.
- [`config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml): Secure TCP only.
- [`config.geo-basic.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-basic.yaml): beginner Geo policy.
- [`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml): nested and direction-sensitive rules.
- [`config.websocket.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.websocket.yaml): WebSocket Signal profile.
- [`config.auth-audit.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.auth-audit.yaml): schema v3 connection-authentication audit canary.
- [`config.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.example.yaml): all sections together.

Always replace example hosts and URLs, then validate logs and real sessions.
