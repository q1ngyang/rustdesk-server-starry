# Fast Relay authorization v1

**English** | [简体中文](FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)

Fast Relay authorization v1 is an additive Starry/Akari extension. HBBS signs
the Relay that HBBS selected; a client cannot select or substitute another
Relay. Official RustDesk clients ignore the unknown high protobuf tag and keep
using the ordinary `relay_server` path.

patch-v1.3.1 preserves the original six-field FastCompat payload and adds the
role-bound fields needed by FastMediaV1 Relay UDP. Missing, malformed, expired,
unsupported, or undeliverable authorization never invalidates the ordinary
reliable HBBR session.

## Outer signalling fields

The overlay adds the same opaque field to two official messages without
renumbering an upstream field:

| Official message | Additive field | Recipient |
| --- | --- | --- |
| `RequestRelay` | `bytes fast_relay_authorization = 64` | controlled/target endpoint |
| `RelayResponse` | `bytes fast_relay_authorization = 64` | controller endpoint |

The value is a libsodium Ed25519 combined signed message:

```text
signed_authorization = signature[64] || protobuf(FastRelayAuthorization)
```

HBBS signs with its existing Ed25519 server key. Akari verifies with the HBBS
public key already used by RustDesk. The canonical additive payload is:

```protobuf
message FastRelayAuthorization {
  uint32 version = 1;
  string session_uuid = 2;
  uint64 expires_at = 3;
  bool allow_fast_compat = 4;
  bool allow_fast_media_v1 = 5;
  uint32 max_bitrate_kbps = 6;
  uint32 relay_udp_protocol = 7;
  string relay_server = 8;
  uint32 relay_udp_port = 9;
  bytes relay_allocation_id = 10;
  uint32 relay_max_datagram = 11;
  uint32 relay_endpoint_role = 12;
}
```

The machine-readable definition is
[`contracts/fast-relay/v1/rendezvous-extension.proto`](../../contracts/fast-relay/v1/rendezvous-extension.proto).
Fields 1–6 retain their patch-v1.3.0 meaning. Old six-field FastCompat grants
remain valid for compatible clients. New clients enforce `relay_server` when
it is present.

## Stable field contract

| Field | Requirement |
| --- | --- |
| `version` | Exactly `1`. Appending fields does not change this value. |
| `session_uuid` | Exact RustDesk session UUID; maximum 128 bytes at HBBS. |
| `expires_at` | Unix seconds; HBBS policy permits a 30–300 second TTL. |
| `allow_fast_compat` | Authorizes the reliable FastCompat media path. |
| `allow_fast_media_v1` | Authorizes AKR1 only when all FastMedia gates pass. |
| `max_bitrate_kbps` | Encoded source ceiling, `1000..=200000` Kbit/s. |
| `relay_udp_protocol` | `1` for AKR1; zero for a compatibility-only grant. |
| `relay_server` | Exact HBBS-selected Relay identity used in ordinary `relay_server`. |
| `relay_udp_port` | Selected HBBR UDP port; non-zero only for FastMedia. |
| `relay_allocation_id` | Fresh non-zero 16-byte identifier, unrelated to the public UUID. |
| `relay_max_datagram` | Complete UDP payload including AKR1, `608..=1400`; default `1200`. |
| `relay_endpoint_role` | `1` controller, `2` target; zero for a six-field-compatible grant. |

For FastMedia, HBBS signs two different byte strings. The target grant in
`RequestRelay` has role 2 and the controller grant in `RelayResponse` has role
1. Both share the UUID, expiry, Relay, UDP port, allocation ID, datagram bound,
and bitrate ceiling. Swapping the grants must fail at HBBR.

## Server-selected Relay and issuance gates

Relay Quality v1 remains authoritative whenever it has a final decision. HBBS
copies that exact Relay into both ordinary `relay_server` and the signed grant.
When quality is disabled, has too few compatible candidates, times out, or
uses a legacy fallback, HBBS may sign only the ordinary GEO/failover Relay that
HBBS itself already selected. A request-provided Relay is never trusted.

HBBS issues any grant only after all of these conditions hold:

1. connection authentication returns the exact `allow` verdict, including in
   audit mode;
2. signalling uses Secure TCP or the configured WebSocket path;
3. the ordinary final Relay is fixed by HBBS and, when present, exactly matches
   the Relay Quality decision;
4. policy TTL, source bitrate, and datagram bounds are valid;
5. the signing key, clock, bounded allocation cache, and per-source signing
   budget are available.

FastMedia additionally requires the selected HBBR to have fresh authenticated
telemetry schema 2, explicit capability `fast_media_relay_udp = 1`, a healthy
UDP endpoint, and both role deliveries. HBBS does not infer capability from a
Starry version string or from client-provided data. If these gates fail, HBBS
may still issue FastCompat when enabled and always retains the reliable Relay.

Retries for the same live UUID and normalized endpoint pair reuse the original
Relay and unexpired role grants. They do not extend expiry or spend another
signature. A UUID conflict, endpoint-pair conflict, final-Relay conflict, or
configuration-generation change fails closed.

## Configuration

Schema v5 keeps both modes independent and default-off:

```yaml
version: 5
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
    relay_max_datagram: 1200
```

FastMedia implies FastCompat for that authorized session, but enabling
`fast_media_v1_enabled` does not rewrite the stored FastCompat switch. A Relay
telemetry endpoint declares `fast_media_udp_port` only beside an authenticated
`/ws/telemetry` secret-file reference. Secret values never appear in YAML.

## Privacy and observability

Control API returns only bounded aggregate counters: grants by role, sessions
by mode, reuse/delivery, unavailable capability, reliable fallback, invalid
selection/configuration, rate limits, and expiry. It never returns a UUID,
allocation ID, endpoint address, token, stage token, signed grant, or media
payload. Logs follow the same rule.

The authorization contract is release-candidate input for patch-v1.3.1. Its
canonical protobuf digest is recorded in the release summary. The AKR1
data-plane contract remains `RELEASE_CANDIDATE_BLOCKED` until the required real
Akari↔HBBS↔HBBR fallback and automatic re-entry integration gate passes.
