# Changelog

**English** | [简体中文](CHANGELOG.zh-CN.md)

This file records Starry overlay changes. The full artifact version combines
the official RustDesk Server version with the Starry patch version, for example
`1.1.16-patch-v1.2.0`.

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
