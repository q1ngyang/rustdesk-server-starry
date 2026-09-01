# Fast Relay authorization v1

**English** | [简体中文](FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)

Fast Relay authorization v1 is an additive Starry/Akari extension that lets
HBBS authorize Akari's `FastCompat` mode only after connection authorization
and Relay-quality selection have succeeded. The first patch-v1.3.0 release
continues to carry media through the existing reliable HBBR stream. It never
authorizes or advertises `FastMediaV1` Relay UDP.

Official RustDesk clients remain compatible. The extension uses an unknown
high protobuf tag, does not change any official field or enum, and leaves the
ordinary `relay_server` flow intact when the extension is absent or rejected.

## Wire binding

The following additive fields are injected into the upstream
`rendezvous.proto` during the overlay build:

| Official message | Added field |
| --- | --- |
| `RequestRelay` | `bytes fast_relay_authorization = 64` |
| `RelayResponse` | `bytes fast_relay_authorization = 64` |

The bytes contain a libsodium Ed25519 combined signed message:

```text
signed_authorization = signature[64] || protobuf(FastRelayAuthorization)
```

HBBS signs with its existing Ed25519 secret key. Akari verifies with the HBBS
public key already distributed for normal RustDesk operation, opens the
combined signed message, and then parses this payload:

```protobuf
message FastRelayAuthorization {
  uint32 version = 1;
  string session_uuid = 2;
  uint64 expires_at = 3;
  bool allow_fast_compat = 4;
  bool allow_fast_media_v1 = 5;
  uint32 max_bitrate_kbps = 6;
}
```

The machine-readable definitions are in
[`contracts/fast-relay/v1/rendezvous-extension.proto`](../../contracts/fast-relay/v1/rendezvous-extension.proto).

## Issuance order and invariants

HBBS issues a grant only when all of these checks succeed, in this order:

1. Connection authentication returns the exact `allow` verdict. In audit mode,
   a request that would be denied does not receive a grant.
2. Signalling uses Secure TCP or the configured WebSocket signalling path.
   Deployments terminating WSS at a reverse proxy must prevent direct public
   access to HBBS's plaintext WebSocket listener.
3. Relay quality protocol v1 has produced the final, source-bound selection
   for the initiating IP, target IP, session UUID, allocation ID, and active
   configuration generation.
4. The active policy is valid: authorization TTL is `30..300` seconds and the
   bitrate ceiling is `1000..200000` Kbit/s.
5. The HBBS signing key and system clock are available and the per-source
   signing rate limit permits a new signature.

The signed payload always has `version = 1`, `allow_fast_compat = true`, and
`allow_fast_media_v1 = false`. A grant is created only after the selected
Relay has been copied into the official `relay_server` field. The exact same
signed byte string is placed in the target's `RequestRelay` and the
controller's `RelayResponse`.

Any failure yields an empty tag 64 field and increments a bounded runtime
counter. It does not reject, delay, or rewrite the standard RustDesk Relay
flow beyond the already-final Relay-quality decision.

## Replay, retry, and privacy controls

- Cache keys bind the session UUID to the normalized initiator and target IPs.
  A UUID cannot be reused by a different endpoint pair while its entry is
  active.
- Response lookup additionally binds the responding target IP and the final
  Relay identity.
- A valid retry reuses the original Relay-quality decision and the exact
  signed authorization bytes; it does not extend expiry or consume another
  signature.
- Entries expire at the earlier practical boundary imposed by the signed
  expiry and the Relay-quality allocation TTL. Maps are bounded by
  `relay_quality.max_allocations`.
- New signatures are limited to 120 per normalized source IP per minute.
- Logs never contain the session UUID, signing key, token, or signed grant.
  They may contain the selected Relay and a truncated opaque allocation label.

Akari must reject a grant if signature verification or protobuf parsing fails,
the version is unsupported, the session UUID differs, `expires_at` has passed,
or `allow_fast_compat` is false. Akari must never infer FastMedia support from
the presence of this extension: `allow_fast_media_v1` is authoritative and is
always false in patch-v1.3.0.

## Configuration and Kessoku contract

Schema v4 adds the default-off policy:

```yaml
fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
```

Enabling it requires Relay quality, connection authentication in `audit` or
`enforce`, and Secure TCP or WebSocket signalling. Kessoku discovers support
through Control API capability `fast_relay_authorization: 1`, includes
`fast_mode` in the normal schema/plan/apply workflow, and records only policy
changes and aggregate runtime counters in audit data. Kessoku must not receive,
store, or log HBBS secret keys, connection tokens, session UUIDs, or signed
authorizations.

`GET /control/v1/relays` exposes bounded `fast_relay` counters for issuance,
reuse, delivery, and fail-closed reasons. Their values are operational
telemetry, not proof that a client entered FastCompat mode.
