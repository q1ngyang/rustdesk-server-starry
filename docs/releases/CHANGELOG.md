# Changelog

**English** | [简体中文](CHANGELOG.zh-CN.md)

This file records Starry overlay changes. The full artifact version combines
the official RustDesk Server version with the Starry patch version, for example
`1.1.16-patch-v1.3.2`.

## patch-v1.3.2 — preview

Release notes: [`RELEASE-NOTES-patch-v1.3.2.md`](RELEASE-NOTES-patch-v1.3.2.md)

### Added

- Frozen FastMedia active-session renewal v1: HBBS-signed controller/target
  grant chains, authenticated Secure TCP/WSS exchange, exact idempotent replay,
  monotonic expiry/cap semantics, and reliable fallback.
- Authenticated Relay telemetry v3 and Control API 1.1 bounded aggregates for
  renewal, replay, admission reservations, remaining TTL, and typed capability
  `fast_media_relay_renewal = 1`.
- Bounded 12-hour-default/24-hour-maximum renewable HBBR allocation lifecycle,
  post-expiry recovery, role transition, 2,048-packet AKF1 replay window, and
  per-role/per-IP/global wire-rate admission.

### Compatibility and status

- Built from the published `1.1.16-patch-v1.3.1` tag. Relay Quality v1,
  schema v5, authorization fields 1–12, AKR1 kinds 1–5, official clients, old
  Akari, and ordinary Native/WS/WSS Relay behavior are unchanged.
- Source, controlled-clock, contract, overlay, lint, test, and build gates
  approve only an opt-in preview. Stable/latest remain blocked on immutable
  real-Akari long-session, cross-network NAT/UDP fault-soak, and hosted
  artifact-provenance evidence.

## patch-v1.3.1 — preview

Release notes: [`RELEASE-NOTES-patch-v1.3.1.md`](RELEASE-NOTES-patch-v1.3.1.md)

### Added

- Role-bound FastRelayAuthorization fields 7–12 and independent default-off
  schema-v5 FastCompat/FastMediaV1 policy.
- HBBR AKR1 UDP cookie/bind/forward/rebind data plane with bounded grants,
  lifetimes, replay, traffic, cleanup, and authenticated telemetry schema 2.
- Server-selected FastCompat/FastMedia grants for quality decisions and safe
  ordinary GEO/failover fallback; clients never select the signed Relay.
- Starry Pairing v1, Control Agent pair/adopt/rotate, bounded Relay enrollment,
  and the separate `starry-relayctl` utility.
- Explicit persistent container/native/DEB identity layouts and a no-side-
  effect schema-v5 to schema-v4 downgrade preview/export with drain and
  90-day certificate-window gates.

### Fixed

- Permit only a legitimate native initial `PunchHoleSent`/`LocalAddr` target
  source-port change under the complete frozen Relay Quality binding; exact
  top-level report/controller-route and conflicting-duplicate checks remain.
- Drain the Control Agent cleanly on termination instead of leaving a
  diagnostic container with an ambiguous forced-stop status.
- Keep generated HBBR test signing keys out of child-process arguments.
- Replace the yanked locked `chacha20 0.10.1` with non-yanked `0.10.2`, and
  make CI reject yanked dependencies as well as vulnerabilities and unsound
  advisories.

### Compatibility and status

- Relay Quality v1 protobuf, digest, scoring, telemetry, hysteresis, privacy,
  and fallback semantics are unchanged.
- Official clients, six-field FastCompat grants, manual Agent/Relay setup, and
  ordinary Native/WSS Relay remain compatible. All new switches default off.
- The protocol-level Akari harness now proves dual-role forwarding, reliable
  fallback, and same-session automatic re-entry against the candidate HBBR.
  The default-off preview is approved; stable publication still requires the
  real two-client GUI/signalling, device/NAT/fault, and production PKI gates.

## patch-v1.3.0 — development

Release notes: [`RELEASE-NOTES-patch-v1.3.0.md`](RELEASE-NOTES-patch-v1.3.0.md)

### Added

- Private additive protobuf v1 fields for Akari Relay-quality capability
  negotiation, candidate offers, dual-end reports, scores, and decisions. All
  quality additions use tags 100+; the ordinary `relay_server` remains
  authoritative for official clients.
- Bounded TCP/WSS HBBR active-probe responses with echoed nonces, exact Starry
  version, active sessions, capacity, current bandwidth, and load basis points.
- Explicit HBBR `relay_probe_protocol=1` / `relay_load_protocol=1` capability
  negotiation, freshness-bounded telemetry, bounded-concurrent HBBS health
  probes, and legacy fallback nodes that never consume quality-candidate slots.
- HBBS selection using effective dual-end RTT, jitter, loss, trusted Relay
  load, missing-report penalties, symmetric network-prefix cache, and
  configurable hysteresis.
- Schema v4 `relay_quality` settings and Control API v1 inventory telemetry for
  Kessoku, without creating a client-to-Control-Agent path.
- A server-enforced allocation report deadline independent of cleanup TTL,
  feasibility validation for ordered samples, stable decision reason codes,
  and aggregate accepted/late/invalid/binding-mismatch counters.
- Default-off schema v4 `fast_mode` policy and additive tag 64 authorization
  bytes in `RequestRelay`/`RelayResponse`. HBBS signs `FastCompat` only after
  exact auth allow and final quality selection, binds/cache-limits the grant,
  and sends identical bytes to both Akari endpoints.
- Control capability `fast_relay_authorization: 1` and bounded `/relays`
  issuance, reuse, delivery, and fail-closed counters. patch-v1.3.0 always
  signs `allow_fast_media_v1 = false` and adds no Relay UDP media path.
- Profile Activation Lease v1 for Akari: 16-byte client activation IDs,
  matching Ready ACKs, 32-byte node-local route leases, and one shared route
  generation authority across Native UDP/TCP and WSS.
- Exact-current `DeactivatePeer`, generation-safe disconnect cleanup, a
  45-second crash-fallback TTL, and a 12-per-30-second verified rapid
  re-registration burst keyed by peer ID, network identity UUID, and public
  key. HBBR data messages remain unchanged.
- Control capability `profile_activation_lease: 1`, aggregate `/relays`
  lifecycle/rejection counters, and exact current-lease `/peers:verify` support
  for per-instance Kessoku reconciliation.

### Compatibility

- Schema v1-v3 remain accepted. Relay quality is disabled unless a schema v4
  configuration enables it and the initiating client opts into protocol v1.
- Official clients continue to receive one legacy `relay_server`; official
  Relay forwarding semantics and ports are unchanged. Unknown tag 64 is
  ignored, while HBBS clears any client-supplied authorization bytes.
- Official/legacy HBBR can be named in `legacy_fallback_relays` for ordinary
  fallback, but missing explicit capability or fresh load always excludes them
  from quality offers. Fewer than two compatible candidates disables the offer
  and preserves the full traditional Geo/failover path.
- Akari force-Relay mode performs the same quality-capable PunchHole preflight
  before `RequestRelay`; a direct RequestRelay without an offer retains the
  legacy selection path.
- FastCompat remains disabled unless schema v4 explicitly enables it together
  with Relay quality, connection authentication, and secure signalling. Any
  missing prerequisite falls back to the standard Relay flow without a grant.
- Official registration messages omit Profile activation fields and retain
  their current path. Akari commits a Profile only after a successful ACK
  echoes its exact activation ID/epoch and supplies a valid lease/generation;
  older servers therefore fail closed to the previous committed Profile.

## patch-v1.2.2 — 2026-08-29

Release notes: [`RELEASE-NOTES-patch-v1.2.2.md`](RELEASE-NOTES-patch-v1.2.2.md)

### Added

- A private, read-only Control API endpoint that verifies an exact RustDesk
  ID/machine-UUID pair against the HBBS registry without exposing peer data.
- Scoped service-only authorization for Kessoku background device discovery.

## patch-v1.2.1 — 2026-08-28

Release notes: [`RELEASE-NOTES-patch-v1.2.1.md`](RELEASE-NOTES-patch-v1.2.1.md)

### Added

- Relay version reporting from the HBBR WebSocket handshake through HBBS
  health observation to the Control API v1 Relay inventory.

## patch-v1.2.0 — 2026-08-20

Release notes: [`RELEASE-NOTES-patch-v1.2.0.md`](RELEASE-NOTES-patch-v1.2.0.md)

### Added

- Schema v3 and strict Ed25519 connection-JWT verification for controller
  `PunchHoleRequest` and direct `RequestRelay` across native TCP, Secure TCP,
  and WSS, with `off`, `audit`, and `enforce` modes.
- Atomic last-known-good JWKS refresh, bounded token/introspection caches,
  mandatory exclusive-CA mTLS for configured remote JWKS and introspection,
  and fail-closed stale/error/subject handling.
- Immutable Relay runtime snapshots and side-effect-free allocation simulation
  with structured decision traces.
- A framed, bounded, loopback-only local control protocol.
- Optional Linux `starry-control-agent` with mandatory mTLS, URI-SAN allowlist,
  short-lived scoped service JWTs, fixed Control API v1, and a safe read-only
  default profile.
- Optimistic, idempotent configuration plan/apply/rollback operations with
  exact-byte ETags, durable audit records, revision history, atomic writes,
  runtime activation acknowledgements, and recovery blocking on uncertainty.
- Versioned OpenAPI 3.1, JSON/UI Schema, JWT/protocol fixtures, Control Agent
  Compose/systemd/DEB assets, and security-focused integration tests.

### Fixed

- Advance `health_snapshot_id` when Relay probe data or readiness changes, so
  Relay inventory and a corresponding allocation simulation identify the
  actual health snapshot rather than only the health-configuration generation.
- Limit the remote JWKS/introspection mTLS HTTP pool idle lifetime to 15
  seconds, preventing reuse of keep-alive connections already closed by
  Kessoku's shorter server idle timeout while preserving last-known-good and
  fail-closed behavior.

### Dependency and build integrity

- The reviewed Cargo graph is locked and copied into the patched upstream tree
  before any locked metadata, test, or release build runs. Upstream and
  `hbb_common` source commits are recorded with candidate inputs.
- Replaced the obsolete SQLx/deadpool SQLite path with bundled
  `tokio-rusqlite`, removed the unpinned Git `reqwest` input, and modernized the
  TLS, WebSocket, JWT, protobuf, and CLI dependency paths.
- The fixed RustSec audit reports no vulnerability, unsound, or yanked crate.
  One upstream-core `sodiumoxide 0.2.7` unmaintained warning remains disclosed
  for explicit release risk review.
- Candidate CI fixes Rust, cross, cross images, advisory data, scanners, base
  images, and every Action to immutable reviewed inputs; Debian packages are
  built twice and compared byte for byte before publication can be approved.

### Compatibility and safety

- Schema v1 and v2 remain accepted; connection authentication is off unless a
  schema v3 document enables it or the deployment `--must-login` floor requires
  enforce mode.
- Invalid reloads retain the active last-known-good generation instead of
  clearing Starry runtime state.
- Authentication uses existing RustDesk protobuf fields; no `.proto` file is
  changed. UDP connection initiation remains unsupported and cannot allocate.
- Native controlled-endpoint registration/heartbeats still use UDP with
  official 1.1.16; TCP/Secure TCP coverage applies to controller initiation.
  A controlled endpoint that must disable UDP needs WSS registration and must
  not treat the upstream TCP `NOT_SUPPORT` response as authentication
  regression or TCP-only registration support.
- HBBR and account/API responsibilities remain unchanged. The Control Agent is
  a separate management component, not an account API or an HBBR proxy.
- Writable Control Agent transactions are supported and release-tested on
  Linux only for patch-v1.2.0; no Windows Agent artifact is published.

## patch-v1.1.0

Release notes: [`RELEASE-NOTES-patch-v1.1.0.md`](RELEASE-NOTES-patch-v1.1.0.md)

### Added

- Optional persistent `/ws/id` signalling compatible with RustDesk client
  identity registration.
- WSS-to-WSS and WSS-to-native signalling with explicit Relay-only routing
  whenever either endpoint uses WebSocket.
- Transport-aware Relay selection for `native`, `wss`, and `mixed` sessions.
- Certificate- and hostname-verified `wss://.../ws/relay` health probes.
- Schema v2 fields for trusted proxy CIDRs, exact Origin allow-listing,
  session/queue/rate limits, and Relay endpoint coverage.
- `websocket-status` management command.
- Optional transport argument for `test-geo <IP_A> <IP_B>
  [native|wss|mixed]`.
- Real-process WebSocket and official-HBBR mixed-transport integration tests.

### Compatibility

- Schema v1 remains accepted and keeps WebSocket Signal disabled.
- Native/P2P behaviour remains the default when the client WebSocket switch is
  off.
- HBBR is not modified by this patch.
- Rolling back to patch-v1.0.0 requires restoring a schema v1 configuration.

## patch-v1.0.0

### Added

- External, strictly validated Starry YAML configuration with safe all-or-none
  fallback to upstream behaviour.
- Ordered Relay selection using both endpoints' country, city, subdivision,
  GeoNames ID, ASN, and ISP facts.
- Nested GEO expressions using `/` (OR), `+` (AND), parentheses, and quoted
  values.
- Symmetric and direction-sensitive endpoint rules.
- MMDB download scheduling, size/marker/readability validation, atomic
  replacement, and last-known-good retention.
- RustDesk-compatible HBBS Secure TCP negotiation and Secretbox transport on
  native `21116/TCP`, with authenticated failure handling and compatible
  plaintext-first-frame fallback.
- Local configuration reload, Relay listing, Geo reload, and two-IP rule test
  management commands.

### Compatibility

- Empty, unparseable, or invalid Starry configuration leaves HBBS on official
  command-line behaviour.
- No matching rule, missing required MMDB, or unavailable rule Relay continues
  through later rules and finally official Relay selection.
- The overlay modifies HBBS only.

## Versioning policy

- `X`: incompatible Starry configuration or behaviour change, or a major new
  feature family.
- `Y`: backwards-compatible feature release.
- `Z`: urgent correction to the current patch line.
- The official prefix changes whenever the pinned upstream RustDesk Server
  release changes, even when the Starry patch version does not.

Read [`Upgrade and rollback`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback)
before changing either part of the version.
