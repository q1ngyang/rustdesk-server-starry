# FastMedia active-session renewal v1

**English** | [简体中文](FAST-MEDIA-RENEWAL-v1.zh-CN.md)

FastMedia active-session renewal v1 is the additive patch-v1.3.2 profile for
the frozen Fast Relay authorization v1 and AKR1 v1 contracts. It extends a
running FastMedia allocation without changing Relay Quality v1, the ordinary
`relay_server`, AKR1 header, or kinds 1 through 5. The canonical protobuf is
[`contracts/fast-media-renewal/v1/rendezvous-extension.proto`](../../contracts/fast-media-renewal/v1/rendezvous-extension.proto),
and exact lifecycle/resource rules are frozen in
[`renewal-contract.json`](../../contracts/fast-media-renewal/v1/renewal-contract.json).

Only HBBS signs grants. HBBR and clients cannot sign or extend them. Failure of
this optional control flow never closes the reliable RustDesk/HBBR connection.

## Capability and compatibility

An HBBR advertises renewal only inside authenticated telemetry v3 as
`fast_media.renewal_protocol = 1`. HBBS exposes the typed aggregate capability
`fast_media_relay_renewal = 1`; it never infers support from a version string.
The existing `fast_media_relay_udp = 1` capability remains unchanged.

- Official clients ignore unknown protobuf fields and messages.
- Old Akari continues with the bootstrap grant and falls back at its original
  expiry.
- patch-v1.3.1 HBBR ignores authorization fields 13–16 and keeps its former
  90/300-second behavior.
- patch-v1.3.2 HBBR accepts old grants but treats them as non-renewed legacy
  allocations.

## Additive authorization fields

`FastRelayAuthorization` version remains `1` and fields 1–12 retain their
frozen meaning. The following fields are appended:

| Tag | Field | Contract |
| ---: | --- | --- |
| 13 | `fast_media_relay_renewal` | `1` for renewal v1; zero means unsupported. |
| 14 | `relay_session_id` | Zero in the bootstrap grant; thereafter the exact non-zero AKR1 session ID. |
| 15 | `renewal_sequence` | Zero at bootstrap; increases by exactly one for each issued role pair. |
| 16 | `previous_authorization_sha256` | Empty at bootstrap; then SHA-256 of that role's complete preceding combined Ed25519 grant. |

Every renewed grant preserves the session UUID digest, HBBS-selected Relay,
allocation ID, UDP protocol/port, FastMedia session ID, role, and datagram
maximum. `expires_at` must increase. `max_bitrate_kbps` may stay equal or
decrease but can never increase. A changed identity, role, Relay, protocol, or
limit fails closed and requires a new allocation.

## Authenticated control exchange

The controller sends `FastMediaRenewalRequest` as Rendezvous `oneof` tag 106
over Secure TCP or WSS. HBBS accepts it only when connection authentication
returns the exact `allow` verdict and all of these match the cached bootstrap
record: the original controller route and normalized IP, session UUID, Relay,
allocation, protocol, datagram limit, current bitrate, sequence, and both role
authorization hashes. The requester role is exactly `1`. An allocation ID by
itself is never authorization.

The first accepted request pins the non-zero FastMedia session ID chosen by the
controller and already accepted by the target. Later changes to that value are
rejected. HBBS signs a new controller grant and a new target grant using its
existing Starry Ed25519 key, then returns `FastMediaRenewalResponse` as `oneof`
tag 107 on the same encrypted controller route. The controller installs its
role grant and carries only the target grant over the already authenticated,
end-to-end encrypted reliable desktop session. No grant is delivered through
Control API, telemetry, or Kessoku.

The response uses stable numeric status codes: `1 OK`, `2 DISABLED`,
`3 UNAUTHENTICATED`, `4 NOT_FOUND`, `5 BINDING_MISMATCH`, `6 EXPIRED`,
`7 TOO_EARLY`, `8 RATE_LIMITED`, `9 UNAVAILABLE`, and `10 INVALID`. Clients do
not display or interpret server free text.

## Idempotency, loss, and ordering

The request contains a random 16-byte `request_id` and SHA-256 of both current
combined grants. On success HBBS advances the pair by exactly one sequence and
caches the exact response. A byte-equivalent retry with the same request ID
returns the identical grant pair while it remains valid, including after the
old pair expires. This is a replay of an already authorized issuance, not
acceptance of an expired grant. A different request at an old sequence, a
different grant at the same sequence, a skipped sequence, or a wrong previous
hash is rejected.

HBBR stores authorization chain state per role. The two roles may temporarily
differ by one sequence. Media continues only while both grants are unexpired
and the mismatch is within the bounded transition window (default 15 seconds).
Once the window ends, UDP media fails closed but the allocation is retained for
bounded recovery and the reliable connection remains usable. A late valid
second role can still converge the pair and resume UDP.

A renewed grant is carried in the existing AKR1 Bind (kind 3), after a fresh
source-bound Cookie. A same-role duplicate is idempotent. A rebind or renewal
updates neither the AKF1 replay window nor accumulated rate-limit state. After
HBBR restart, an unexpired HBBS-signed renewed grant can recreate the same
allocation/session; expired grants cannot.

## Timers and fallback

The signed grant TTL remains the schema-v5 setting in `30..=300` seconds.
Clients start renewal at:

```text
expires_at - min(60, max(30, floor(ttl / 3)))
```

HBBS returns `TOO_EARLY` before that point. `fallback_before` is exactly ten
seconds before expiry. If both new grants have not become usable by that
deadline, Akari preserves the last frame and returns media to FastCompat before
the old grant expires. Retries use bounded backoff and never stop input,
control, or the reliable desktop stream.

For renewable allocations HBBR does not delete a fully bound, active session
at the original bootstrap expiry or at 300 seconds from creation. It still
enforces half-bind TTL, idle TTL, allocation/session counts, incremental
cleanup, expired-role media denial, a post-expiry recovery window (default 30
seconds), and an absolute session lifetime (default 12 hours, bounded to 10
minutes through 24 hours). There is no immortal allocation.

## Replay and admission

The HBBR AKF1 replay window is 2048 packets (`32 × u64`). It is preserved over
renewal and rebind, rejects duplicates and packets older than the window, and
rejects a forward jump greater than 1,048,576. Cross-word shifts are part of
the frozen behavior.

For a signed source cap, HBBR computes:

```text
wire_kbps = ceil(source_kbps × 1.45)
wire_bytes_per_second = ceil(wire_kbps × 1000 / 8)
```

Each first role bind reserves that rate against per-role, normalized source-IP,
and global ledgers before acceptance. Same-NAT roles and sessions add together;
a cross-IP rebind moves the reservation atomically; a renewal may release but
not increase it. HBBS uses fresh telemetry plus a 90% reservation headroom and
its bounded per-Relay/IP ledger to lower the signed source cap before issuance.
Both grants and the response contain the actual cap. Thus the default 32 MiB/s
per-IP bucket cannot receive a 200 Mbit/s source grant whose wire allowance is
290 Mbit/s. Authorizing first and relying on stable throttling is forbidden.

## Privacy-safe observability

Telemetry v3 and Control API expose fixed aggregate counters for successful,
idempotent, invalid, binding, sequence, expired, rate, replay, and admission
outcomes; active/reserved totals; minimum remaining TTL; and allocations
approaching expiry. They never expose a UUID, client IP, allocation or request
ID, nonce, token, stage token, grant, or media payload. Kessoku can consume the
typed aggregates but is outside both the signing and media paths.

