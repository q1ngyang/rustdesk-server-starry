# Configuration reference

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)

Starry reads `starry/config.yaml` relative to the HBBS data directory. In the
container this is `/root/starry/config.yaml`, because `/root` is the persistent
data mount. On first start HBBS also creates
`starry/config.example.yaml` as a local reference.

The parser rejects unknown fields, duplicate list values, invalid ranges, and
cross-references to Relays that are not declared in `relay_servers`. A missing,
empty, or invalid file does **not** partially enable Starry features: HBBS logs
the error and keeps upstream-compatible behaviour. Treat that fallback as a
safety property, not as a reason to ignore startup logs.

## Document version and feature gates

| Field | Required | Accepted values | Meaning |
| --- | --- | --- | --- |
| `version` | Yes | `1`, `2` | Configuration schema. Use `2` for new deployments. |

Schema `1` supports Relay, Secure TCP, MMDB, and Geo settings. It rejects the
`websocket_signal` section. Schema `2` adds optional WebSocket Signal settings.
Unknown top-level and nested keys are rejected so that misspellings cannot
silently change a deployment.

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
| `path` | `mmdb/GeoLite2-Country.mmdb`, `mmdb/GeoLite2-City.mmdb`, or `mmdb/GeoLite2-ASN.mmdb` | Must not be empty. Relative paths resolve under the HBBS working/data directory. |
| `url` | empty | Optional `http://` or `https://` download URL. Empty means local-file management. Prefer HTTPS and a source you are licensed to use. |

Downloads are written to a temporary file, checked for the minimum size,
MaxMind marker, and reader compatibility, then atomically replace the target.
On failure the previous readable file remains in place. The image does not
contain GeoLite2 data and does not provide a database licence.

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

This section requires `version: 2` and is opt-in.

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
        url: wss://relay-asia-1.example.com/ws/relay
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
| `endpoints[].url` | None | Required unique URL: `wss://` plus a DNS hostname and the exact `/ws/relay` path; no credentials, query, or fragment. |

When `enabled: true`, endpoint Relay names must cover `relay_servers` exactly.
The health probe verifies the WSS/TLS path used for allocation; it does not
replace a two-client remote-control test. See
[Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS).

## Reload behaviour

After editing the file, reload it inside the HBBS network namespace:

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
```

Reload is atomic at the document level: either the complete valid file becomes
active, or an empty/invalid file disables Starry configuration and returns HBBS
to upstream behaviour. The previous Starry state is **not** retained on an
invalid reload. Restore the last known-good file or correct the logged error,
reload again, and confirm acceptance. For a deterministic maintenance cutover,
especially after changing transport settings, restart HBBS and repeat the full
verification checklist.

## Ready-made profiles

- [`config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml): Secure TCP only.
- [`config.geo-basic.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-basic.yaml): beginner Geo policy.
- [`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml): nested and direction-sensitive rules.
- [`config.websocket.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.websocket.yaml): WebSocket Signal profile.
- [`config.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.example.yaml): all sections together.

Always replace example hosts and URLs, then validate logs and real sessions.
