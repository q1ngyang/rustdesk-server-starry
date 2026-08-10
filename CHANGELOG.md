# Changelog

**English** | [简体中文](CHANGELOG.zh-CN.md)

This file records Starry overlay changes. The full artifact version combines
the official RustDesk Server version with the Starry patch version, for example
`1.1.16-patch-v1.1.0`.

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
