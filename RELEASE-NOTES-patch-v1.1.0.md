# patch-v1.1.0 release notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.1.0.zh-CN.md)

patch-v1.1.0 adds an opt-in persistent WebSocket Signal path for RustDesk
clients on networks where native signalling is unavailable, while retaining
native signalling and P2P as the normal faster path.

## Component scope

- Starry code changes are confined to HBBS.
- Official HBBR already supplies native `21117/TCP` and `/ws/relay`; this patch
  verifies and uses that contract without modifying HBBR.
- No account/API server is included. A third-party API remains a separate
  deployment and security decision.

## Added

- Persistent `/ws/id` registration using the same RegisterPk identity checks as
  native registration.
- Single-writer WebSocket sessions with bounded outbound queues,
  generation-safe route replacement, heartbeat, idle timeout, global and
  per-effective-IP limits, and registration rate limiting.
- WSS-to-WSS and WSS-to-native signalling. If either endpoint uses WSS, HBBS
  explicitly forces Relay and does not advertise a P2P path that the restricted
  endpoint cannot use.
- Transport-aware Relay selection:
  - `native` requires the official HBBS online Relay state;
  - `wss` requires a certificate-valid healthy `/ws/relay`; and
  - `mixed` requires both states on the same Relay node.
- Normal DNS, TCP, TLS chain, hostname, and exact WebSocket Upgrade validation
  for every `wss://.../ws/relay` health endpoint. TLS bypass, ping, and plain
  HTTPS 200 are not accepted substitutes.
- Schema v2 validation for trusted proxy CIDRs, exact Origin allow-listing,
  endpoint coverage, frame/queue/session/rate limits, and timing relationships.
- Loopback management commands:
  - `websocket-status` (`ws`); and
  - `test-geo <IP_A> <IP_B> [native|wss|mixed]` (`tg`).
- Automated tests using a real HBBS process, a runtime-generated CA and
  hostname-valid certificate, plus an unmodified official HBBR for mixed Relay
  traffic.

## Behaviour and compatibility

- Existing schema `version: 1` documents remain valid. WebSocket Signal is
  disabled and a v1 document containing `websocket_signal` is rejected.
- Schema `version: 2` is required to configure WebSocket Signal.
- Enabling `websocket_signal.enabled` does not change any client setting. Each
  client independently chooses whether to use WebSocket.
- A client with WebSocket off keeps the patch-v1.0.0 native/P2P path.
- A client with WebSocket on uses WSS signalling and Relay-only data transport.
- One WSS and one native endpoint use the same HBBR node and Relay UUID through
  different transports.
- Every `relay_servers` item must have exactly one `relay_health.endpoints`
  entry while WebSocket Signal is enabled.
- Native RustDesk clients may omit Origin. If an Origin is present, it must
  exactly match `allowed_origins`.
- Forwarded client addresses are trusted only when the direct TCP peer belongs
  to `trusted_proxies`.

## Upgrade from patch-v1.0.0

1. Back up the complete HBBS data directory and current schema v1 file.
2. Upgrade the binary or image while leaving the v1 file in place. Native
   behaviour should remain unchanged.
3. Verify native registration, authenticated Secure TCP, P2P, and native Relay.
4. Deploy certificate-valid Nginx `/ws/id` and every Relay `/ws/relay` endpoint.
5. Create a schema v2 document with exact endpoint coverage, but initially keep
   `websocket_signal.enabled: false`.
6. Reload and confirm that the configuration is accepted.
7. Enable WebSocket Signal, reload again, and inspect `websocket-status`.
8. Test WSS-to-WSS and both mixed directions with real clients before broad
   rollout.

The complete procedure and rollback gates are documented in
[`Upgrade and rollback`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback).

## Rollback

To return to patch-v1.0.0:

1. disable WebSocket on affected clients;
2. restore the schema v1 configuration backup;
3. restore the previous binary or image; and
4. verify native registration, Secure TCP, and a real remote-control session.

patch-v1.0.0 does not understand schema v2. Restoring only the old image while
leaving a v2 document causes Starry configuration to be rejected and HBBS to
fall back to upstream behaviour.

## Validation evidence

The release gate replayed the overlay twice on a clean official 1.1.16 source
tree, required an unchanged second application and clean `git diff --check`,
ran locked library and binary checks, exercised a real HBBS WebSocket process,
and tested bidirectional mixed native/WSS payloads through unmodified official
HBBR. Compose and container smoke tests cover both published architectures.

After publication, the maintainer reports normal use in the tested deployment
and successful client-side switching between native and WebSocket modes. This
is an operational report for that deployment, not a warranty for different
DNS, proxy, certificate, API, client, or network environments.

## Security notes

- Terminate public WSS at a trusted reverse proxy with a valid certificate.
- Do not expose the HBBS management interface publicly.
- Restrict plain backend ports `21118` and `21119` to the intended proxy path.
- Never disable TLS verification to make Relay health appear successful.
- Keep full Peer IDs, tokens, raw client addresses, API secrets, and the HBBS
  private identity key out of documentation and status output.

## Documentation

- [Container image usage](CONTAINER.md)
- [Configuration reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference)
- [Operations and verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification)
- [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting)
