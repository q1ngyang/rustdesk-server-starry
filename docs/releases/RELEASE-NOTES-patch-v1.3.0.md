# patch-v1.3.0 release notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.3.0.zh-CN.md)

patch-v1.3.0 adds three Akari-focused features while preserving the official
RustDesk client wire path: active candidate-Relay quality scoring, signed
`FastCompat` Relay authorization for Akari's P2P fast mode, and generation-safe
Profile Activation Leases with bounded rapid re-registration. This candidate
remains blocked from publication until exact-commit CI and release review
approve it.

## What changed

HBBS can retain up to five eligible Relay candidates after transport, health,
GEO-rule, and configured-order filtering. The default `adaptive` strategy asks
both Akari endpoints to probe only the GEO primary first. HBBS accepts that
primary when its server-interpreted score and loss meet the configured
good-enough thresholds; otherwise it concurrently expands both endpoints to
the remaining candidates. `strategy: eager` remains available for explicit
all-candidate probing. Only HBBS combines the reports with trusted HBBR load,
chooses one Relay, and writes the same result to the ordinary `relay_server`
and decision extension.

Each stage has a fresh token and a server deadline. Reports bind allocation,
stage, token, endpoint role, configuration generation, signalling route, and
both endpoint IPs. Duplicate reports are idempotent, old-stage or replayed
reports cannot change a decision, and a bounded HBBS timer completes partial
or fallback decisions at the deadline. Successful P2P explicitly cancels the
allocation without waiting for Relay probes. Force Relay, symmetric NAT, WSS,
and mixed paths wait for the final server decision before connecting HBBR.

The default score is higher-is-better on `0..10000`:

- RTT: 40%; for two successful endpoint reports, effective RTT is
  `(2 × max(RTT_A, RTT_B) + RTT_A + RTT_B) / 4`;
- jitter: 20%, using the worse endpoint;
- loss: 25%, using the worse endpoint; and
- Relay load: 15%, using HBBS-observed HBBR telemetry.

Missing endpoint measurements receive a configurable penalty. A symmetric
IPv4 `/24` or IPv6 `/56` cache plus a score hysteresis threshold avoids Relay
flapping. Full client IP addresses are not used as cache keys.

HBBR now accepts one bounded `RelayProbeRequest` on either native framed TCP or
WebSocket Relay transport and returns a nonce-bound `RelayProbeResponse`.
Public `/ws/relay` handshakes and probe responses expose only the exact Starry
version plus probe/load protocol capability versions; they never contain the
detailed load snapshot. HBBS obtains active sessions, pending pairs, bandwidth
EMA, capacity, draining/admission state, and aggregate counters only from the
authenticated `/ws/telemetry` channel. The upstream session pairing and
byte-forwarding loop remains intact.

HBBR enforces global and source-IP probe budgets and publishes only bounded
malformed, unsupported, rate-limited, and successful counters. It also treats
`STARRY_RELAY_MAX_SESSIONS` as a real admission limit: capacity or draining
rejects new pairs without terminating existing sessions. Every signed schema-v1
snapshot carries a process instance ID, monotonic sequence, observation time,
uptime, and admission-rejection count so HBBS can fail closed on replay, stale
data, or an instance restart.

Mixed sessions receive endpoint-specific probe URLs under one allocation: the
native side measures native TCP while the WebSocket side measures WSS.

After connection authentication and the final Relay-quality decision, HBBS
can sign a short-lived `FastRelayAuthorization` with its existing Ed25519 key.
The target's `RequestRelay` and controller's `RelayResponse` receive the exact
same signed bytes. Grants are bound in server memory to the session UUID,
initiator IP, target IP, selected Relay, allocation, and active configuration
generation. Valid retries reuse the original decision and signature without
extending expiry.

This release authorizes only `FastCompat` over the existing reliable HBBR
stream. Every signed grant explicitly sets `allow_fast_media_v1 = false`;
Relay-side FastMedia UDP is not implemented or advertised in patch-v1.3.0.

Akari registration can now include a 16-byte random activation ID and a
monotonic epoch. A successful Ready ACK echoes both and returns an opaque
32-byte, node-local route lease plus the HBBS route generation. Akari commits
the Profile switch only after all values match. Native UDP/TCP and WSS share
one generation authority; explicit `DeactivatePeer`, WSS reader exit, idle
drain, delayed packets, and A→B→A switching can remove only their exact current
route. A 45-second lease TTL remains the crash fallback.

Verified re-registration requires an exact stored peer ID, network identity
UUID, and public key. It receives a bounded 12-new-lease burst per rolling 30
seconds while the global/IP blocker remains active. The optimization neither
deletes registration limits nor relies on a shorter TTL. HBBR receives no new
Profile-activation data message.

## Compatibility contract

All Starry fields are additive protobuf extensions: Relay-quality messages use
tags `100+`, while the opaque signed authorization uses unoccupied tag `64` in
`RequestRelay` and `RelayResponse`. No official field is renumbered or retyped.
Compatibility behaves as follows:

| Initiating client | Controlled client | Behaviour |
| --- | --- | --- |
| Official | Official or Akari | Legacy HBBS selection and one ordinary `relay_server`; no quality allocation or FastCompat grant is created. |
| Akari protocol v1 | Official | Akari probes the primary and, if needed, the remaining candidates alone; the missing peer report is penalized and marked `partial`. The selected ordinary `relay_server` works for the official peer. The official peer ignores tag 64. |
| Akari protocol v1 | Akari protocol v1 | Both endpoints probe the primary, expand together only when required, and receive one HBBS decision; when all authorization gates pass HBBS forwards the same signed FastCompat grant to each side. |
| Akari without an offer | Any | Direct `RequestRelay` remains on the legacy path. Force-Relay mode should first perform the quality-capable PunchHole preflight. |

Official HBBR implementations do not understand the probe oneof and simply
close that short probe connection. They can remain an explicitly configured
ordinary `relay_server` fallback, but never enter a quality offer or consume
`max_candidates`. Production quality pools must use HBBR that explicitly
advertises `relay_probe_protocol=1` and `relay_load_protocol=1`; Starry version
strings are inventory only and never imply capability.

The normative field binding and message definitions are in the
[Relay Quality v1 contract](../reference/RELAY-QUALITY-PROTOCOL-v1.md) and
[Fast Relay Authorization v1 contract](../reference/FAST-RELAY-AUTHORIZATION-v1.md).
The matching-ACK, deactivation, multi-node, rollout, and rollback rules are in
the [Profile Activation Lease v1 contract](../reference/PROFILE-ACTIVATION-LEASE-v1.md).

## Configuration

Relay quality requires schema v4 and is off by default:

```yaml
version: 4
relay_servers:
  - relay-asia-1.example.com:21117
  - relay-asia-2.example.com:21117

websocket_signal:
  relay_health:
    endpoints:
      - relay: relay-asia-1.example.com:21117
        url: wss://relay-asia-1.internal.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry
      - relay: relay-asia-2.example.com:21117
        url: wss://relay-asia-2.internal.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry

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

fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
```

Weights must all be positive and sum to 10000. Enabling quality requires at
least two non-legacy Relays, one unique telemetry endpoint for every quality
Relay, and `max_candidates >= 2`. Configure every HBBR
with a realistic session capacity, for example:

```sh
STARRY_RELAY_MAX_SESSIONS=10000
```

`TOTAL_BANDWIDTH` remains the upstream HBBR capacity setting in Mbit/s. HBBS
uses load only from its own certificate-verified and request-authenticated HBBR
snapshot. The YAML stores an absolute secret-file path, never the secret value;
internal mTLS remains preferred, while the HMAC protects deployments whose
reverse proxy terminates TLS. Missing, legacy, incomplete, replayed, or stale
telemetry excludes a Relay from quality offers; client-provided load is never
trusted for selection. `legacy_fallback_relays` is the explicit escape hatch
for an ordinary legacy fallback. Configured Relay health probes keep running
for quality collection even when HBBS WebSocket Signal is disabled; this does
not make WSS/mixed allocation eligible. See the
[Relay Telemetry v1 contract](../reference/RELAY-TELEMETRY-v1.md) and
[security/operations guide](../wiki/Relay-Telemetry-Operations.md).

FastCompat is independently off by default. Enabling it requires Relay quality
to be enabled, connection authentication in `audit` or `enforce`, and Secure
TCP or WebSocket signalling. A valid token must still produce the exact
`allow` verdict in audit mode. TTL is limited to `30..300` seconds and bitrate
to `1000..200000` Kbit/s. If a reverse proxy terminates WSS, block direct
public access to HBBS's plaintext WebSocket listener.

## Kessoku and Control API

Control API v1 keeps its least-privilege boundary and advertises
`relay_quality: 1`, `relay_active_probe: 1`, `relay_probe_protocol: 1`,
`relay_load_protocol: 1`, `relay_telemetry_schema: 1`, and
`fast_relay_authorization: 1`, plus `profile_activation_lease: 1`.
`GET /control/v1/relays` includes each HBBR capability version, telemetry
schema/instance/sequence/uptime/restart state, active/pending/bandwidth/capacity,
draining/admission counters, observation time/age/stale state, aggregate
accepted/late/invalid/binding-mismatch report counters, fixed offer/fallback
reason counters, current strategy, primary probes/accepts, expansions, P2P
cancellations, estimated probe attempts saved, expanded decisions/timeouts,
bounded per-Relay selection counters, and bounded Fast Relay
issuance/reuse/delivery/fail-closed counters. Its `profile_activation` object
adds aggregate lease, ACK, renewal, cleanup, stale, rate, TTL, and capacity
counters without exposing peer or lease secrets. Kessoku can reconcile an
exact live activation per HBBS instance through `/peers:verify`.

Kessoku may configure and observe this state. Probe candidates and reports
travel only through HBBS signalling; Akari never calls the Control Agent and
Kessoku never proxies Relay data. Schema/plan/apply/audit must never expose or
persist the HBBS signing key, connection tokens, session UUIDs, or signed
grants.

Control Agent plans classify every `/relay_quality` change as at least
`medium`; security-sensitive changes with an existing higher classification
remain `high` or `critical`.

## Upgrade and rollback

1. Provision internal TLS/mTLS policy and read-only secret files while Relay
   quality stays disabled.
2. Upgrade HBBR first. Confirm public probes contain no detailed load and each
   quality Relay serves authenticated schema-v1 telemetry with explicit
   probe/load capabilities, a stable instance, and an increasing sequence.
3. Upgrade HBBS next with the existing schema v3 (or with quality disabled),
   then verify fresh telemetry and privacy-safe local Control inventory.
4. Upgrade Control Agents last. They read HBBS local control only and never
   connect to HBBR. Upgrade Kessoku only for the separate Profile-activation
   capability; it does not scrape HBBR or proxy telemetry.
5. Canary Akari's matching-ACK Profile switch across Native/WSS and multiple
   HBBS nodes; then submit a schema v4 configuration with Relay quality enabled
   through validate/plan/apply.
6. Move connection authentication through audit to enforce, verify secure
   signalling, and only then enable `fast_mode.relay.fast_compat_enabled` in a
   separate planned change.
7. Keep the normal direct/P2P and official-client acceptance tests in the
   rollout gate.

Before rolling back to patch-v1.2.0, restore a schema v3 document without
`fast_mode` or `relay_quality`; patch-v1.2.0 rejects schema v4 and both feature
fields. Disable FastCompat first, wait longer than the configured authorization
TTL, and then remove the fields. Cached quality decisions and signed grants are
process memory only and require no data migration.

For an HBBR-only rollback, first disable Relay Quality (or move the affected
node into `legacy_fallback_relays`) and wait at least `report_timeout_ms` before
replacing HBBR. This prevents a capability-disappearing node from remaining in
an in-flight offer. HBBS rollback comes last.

For Profile activation rollback, first stop new Akari switches, retain the last
committed Profile, send best-effort deactivation to every issuing HBBS, and
wait at least 45 seconds plus WSS idle/drain. The state is process-local and
requires no database migration. An older server returns no matching enhanced
ACK, so conforming Akari clients keep their previous Profile.

## Verification included

- Legacy authentication fixtures decode with every quality field at its
  protobuf default.
- Enhanced offer/report/decision messages and active probe envelopes round
  trip through generated protobuf code.
- A real HBBR process exposes only capability/version metadata publicly,
  returns signed detailed telemetry only after authentication, echoes nonce
  over native TCP, WS, and TLS-terminated WSS probes, rate-limits probes, and
  still bridges native-to-WebSocket and WebSocket-to-native bytes.
- Real-process lifecycle checks cover pending-to-active pairing, capacity
  admission, draining, existing-session survival, telemetry sequence, and
  process-instance restart semantics.
- Unit tests cover GEO-primary acceptance without expansion, either-endpoint
  poor-primary expansion, identical dual-endpoint decisions, eager mode,
  official-peer partial reports, stage replay/deadlines, P2P cancellation and
  resource bounds, plus dual-path RTT/jitter/loss/load scoring, legacy-first GEO
  candidate limits, telemetry staleness, bounded health concurrency, late
  reports, impossible deadline configurations, reload retention, schema v4
  gates, and weight validation.
- A real HBBS/WSS signal test covers final quality selection, server-side
  replacement of untrusted tag 64 bytes, Ed25519 verification, FastMedia=false,
  identical two-party delivery, and exact retry reuse.
- Real-process tests cover Native UDP/TCP, WSS old-reader cleanup, same-activation
  WSS retry, delayed cross-transport heartbeat rejection, reordered
  renewal/deactivation, A→B→A, legacy registration defaults, exact current
  deactivation, and independent leases on two HBBS nodes. Unit tests enforce
  the verified 12-per-30-second burst and capacity boundaries.
- The overlay is applied twice and its digest includes the modified protobuf
  source, contracts, and `PATCH_VERSION`.
- Publication creates or verifies an annotated tag bound to the exact release
  commit and never moves it. The checksummed `STARRY-RELEASE-SUMMARY.json`
  binds the published image index/linux-amd64 manifests to the exact Control
  OpenAPI, config schema/UI schema, frozen Relay Quality protocol, and Relay
  telemetry schema digests for Kessoku pinning.
