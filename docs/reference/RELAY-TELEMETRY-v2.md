# Relay Telemetry v2

**English** | [简体中文](RELAY-TELEMETRY-v2.zh-CN.md)

Relay Telemetry v2 is the patch-v1.3.1 authenticated HBBR snapshot. It retains
Relay Telemetry v1 transport, request/response HMAC, freshness, instance,
sequence, load, capacity, draining, admission, and public-probe privacy
semantics. It adds one required bounded `fast_media` object. The canonical JSON
Schema is
[`contracts/relay-telemetry/v2/telemetry.schema.json`](../../contracts/relay-telemetry/v2/telemetry.schema.json).
The frozen non-secret fixture is
[`telemetry.example.json`](../../contracts/relay-telemetry/v2/telemetry.example.json).
Both files are bound by the patch-v1.3.1 contract-candidate summary while the
runtime release remains blocked.

## Trust boundary

HBBR never connects to Kessoku or Control API. HBBS actively pulls
certificate-verified `/ws/telemetry`, authenticates it through the existing
secret-file HMAC (or an internal mTLS boundary), validates schema and sequence,
and decides freshness. The Control Agent returns only HBBS's privacy-safe
aggregate view. A client-supplied probe or report is never a load source.

The public `/ws/relay` upgrade and `RelayProbeResponse` continue to expose only
protocol capability and optional version metadata. They contain no active,
pending, bandwidth, capacity, draining, admission, or FastMedia runtime data.

## Base fields retained from v1

Every schema-2 snapshot contains a process instance ID, monotonic sequence,
observation time in Unix milliseconds, uptime, exact version, explicit probe
and load protocol versions, load basis points, paired active sessions,
unpaired pending legs, enforced session capacity, bandwidth EMA in bit/s and
its alpha, configured bandwidth capacity, draining/admission state, and
bounded probe/authentication counters.

HBBS accepts only an increasing sequence/uptime for one instance. A changed
instance is a restart, clears the old sequence expectation, and is exposed as
such. A future timestamp outside the clock allowance fails validation. An old
valid snapshot remains inventory evidence but becomes stale at
`max_telemetry_age_seconds` and is not eligible for Relay Quality or FastMedia
authorization.

## Required `fast_media` object

| Field | Meaning |
| --- | --- |
| `protocol` | Exact AKR1 protocol version; `1`. |
| `enabled` / `healthy` / `udp_port` | Listener configured, currently serving, and bound port. Healthy implies enabled and non-zero port. |
| `active_allocations` | In-memory AKR1 allocations with at least one role. |
| `active_streams` | Allocations whose controller and target are both bound. |
| `hello_accepted` / `cookie_rejected` | Stateless-cookie aggregate outcomes. |
| `bind_succeeded` / `bind_rejected` / `grant_rejected` | Role installation and authorization outcomes. |
| `role_mismatch` / `session_mismatch` / `allocation_mismatch` | Fixed, bounded rejection classes. |
| `rebinds` | Accepted same-role source-tuple migrations. |
| `forwarded_packets` / `forwarded_bytes` | AKF1 forwarded after removing AKR1. |
| `dropped_packets` / `rate_limited` / `replay_rejected` | Bounded data-plane rejection aggregates. |
| `expired_allocations` / `listener_failures` | Lifecycle cleanup and listener supervision events. |

These values never contain a Relay allocation ID, session/connection UUID,
nonce, address, token, grant, AKF1 bytes, or media content. Control API uses a
fixed field set and configured Relay keys only, so metric dimensions cannot be
created by untrusted client input.

## Eligibility and downgrade

HBBS treats `fast_media_relay_udp = 1` as usable only when schema 2 is fresh,
the object is enabled and healthy, its UDP port equals the configured endpoint,
and the Relay identity exactly matches the final server selection. Official,
legacy, stale, schema-1, or unhealthy HBBR has null/unavailable FastMedia
status and remains eligible only for ordinary Relay fallback.

Rolling back a Relay to patch-v1.3.0 restores telemetry schema 1 through its
existing secret file. HBBS must disable FastMedia for that node before the
rollback; ordinary load telemetry and Relay service remain available. Roll HBBR
first on upgrade, then HBBS and Control Agent. Roll Control Agent first on a
management-only rollback, and HBBS before HBBR only after FastMedia has drained.
