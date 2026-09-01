# Profile Activation Lease v1

**English** | [简体中文](PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)

This document is the wire and rollout contract for Starry patch-v1.3.0 and
Akari profile switching. It extends HBBS registration only. HBBR session
pairing, authentication, and data-forwarding messages are unchanged.

## Goals and invariants

An Akari client can prepare another profile while its current profile remains
usable, but it MUST commit the local switch only after the intended activation
receives a matching Ready ACK. A stale socket, delayed datagram, superseded
WebSocket reader, or old profile can never remove the newest route.

The following tuple names one activation on one HBBS process:

```text
(peer_id, network_identity_uuid, activation_epoch, activation_id,
 route_lease, route_generation)
```

- `activation_id` is 16 cryptographically random client bytes, newly generated
  for each activation attempt.
- `activation_epoch` is a non-zero, monotonically increasing value for the
  peer/profile identity. A higher epoch supersedes a lower one.
- `route_lease` is an opaque 32-byte server secret. Clients compare and return
  it verbatim; they do not derive or log it.
- `route_generation` is a non-zero, process-local server sequence shared by
  Native UDP, Native TCP, and WSS routes.
- A route lease is local to the HBBS process that issued it. It is neither a
  cluster token nor an HBBR credential.

## Additive protobuf fields

The authoritative injection fragment is
[`contracts/profile-activation/v1/rendezvous-extension.proto`](../../contracts/profile-activation/v1/rendezvous-extension.proto).
No official field is renumbered, retyped, or reused.

| Existing message | Tag | Field | Direction and meaning |
| --- | ---: | --- | --- |
| `RegisterPk` | 61 | `activation_epoch` (`uint64`) | Akari registration intent. |
| `RegisterPk` | 63 | `activation_id` (`bytes`) | Exactly 16 random bytes. |
| `RegisterPkResponse` | 60 | `route_generation` (`uint64`) | Generation committed by HBBS. |
| `RegisterPkResponse` | 61 | `activation_epoch` (`uint64`) | Exact echo of the accepted intent. |
| `RegisterPkResponse` | 62 | `route_lease` (`bytes`) | New or exactly reused 32-byte lease. |
| `RegisterPkResponse` | 63 | `activation_id` (`bytes`) | Exact echo of the accepted intent. |
| `RegisterPeer` | 60 | `route_generation` (`uint64`) | Lease-bound heartbeat/renewal. |
| `RegisterPeer` | 61 | `activation_epoch` (`uint64`) | Lease-bound heartbeat/renewal. |
| `RegisterPeer` | 62 | `route_lease` (`bytes`) | Lease-bound heartbeat/renewal. |
| `RegisterPeer` | 63 | `activation_id` (`bytes`) | Lease-bound heartbeat/renewal. |

The Ready ACK is a successful `RegisterPkResponse`. An enhanced response is
valid only when its result is `OK`, its epoch and activation ID exactly match
the outstanding request, its lease is 32 bytes, and its generation is non-zero.
Akari MUST NOT publish the new Profile as active before all of those checks
pass. A timeout, legacy/default-valued response, mismatched echo, malformed
lease, or error result leaves the old Profile committed.

A retransmission of the same current epoch and activation ID may receive the
same lease and generation. Reconnection after confirmed transport cleanup, or
a higher epoch, receives a new lease and generation. A reused epoch with a
different activation ID or public key is stale and is rejected.

## Explicit deactivation

`RendezvousMessage` adds `DeactivatePeer` at oneof tag 62 and
`DeactivatePeerResponse` at tag 63. The request carries the entire tuple:

| `DeactivatePeer` field | Tag | Requirement |
| --- | ---: | --- |
| `id` | 1 | Non-empty peer ID. |
| `network_identity_uuid` | 2 | Exactly 16 bytes. |
| `activation_epoch` | 3 | Non-zero and current. |
| `activation_id` | 4 | Exactly 16 bytes and current. |
| `route_lease` | 5 | Exactly 32 bytes and current on this node. |
| `route_generation` | 6 | Non-zero and current. |

HBBS sets `deactivated: true` only after it atomically verifies the complete
tuple, removes that exact Native or WSS route, and retires the activation.
Otherwise it returns `false` (or the transport closes on a malformed request)
without changing the current route. The response echoes the request epoch,
activation ID, and generation so the caller can correlate reordered replies.

For WSS, explicit deactivation detaches only the current generation before the
ACK is written. Normal reader exit and idle drain compare both the shared route
generation and a server-internal WSS connection ID. That connection ID is not
part of the wire contract; it preserves `remove_if_current` when a retry of the
same activation deliberately reuses its lease/generation. Native route removal
compares the same generation and exact socket address. Consequently an A1
disconnect/deactivation arriving after A1→B→A2 cannot remove A2.

## Legacy compatibility

Official clients omit every extension field and continue through the existing
registration and routing path. Their responses leave the new fields at
protobuf defaults. Once an enhanced lease for an ID is active in a process, a
legacy registration cannot silently replace that leased route.

Official or older Starry servers ignore the unknown fields in an enhanced
registration. They do not return a matching Ready ACK, so a conforming Akari
client does not commit the switch. They also ignore the unknown deactivation
oneof arm. This is a fail-closed feature fallback, not a requirement to fork
the official client protocol.

## Lease lifetime and disconnect cleanup

Successful heartbeats touch only the exact current route. A Native heartbeat
from another socket address, or one presented while WSS owns the route, gets
`request_pk: true` and cannot migrate the route; transport migration requires a
full verified `RegisterPk`. Explicit disconnect cleanup clears the matching
route and lease immediately; it keeps the activation record briefly so the
same activation can reconnect safely with a new lease/generation. WSS server
drain uses the same generation-and-connection-safe cleanup.

The 45-second route lease TTL is a crash and lost-disconnect fallback, not the
normal switch mechanism. Expiration is enforced on access and periodic bounded
maintenance. Empty records are retained for at most 15 minutes to reject stale
epochs while bounding memory. The registries for peer locks, leases, enhanced
peer IDs, and burst identities each have a 100,000-entry ceiling; capacity
exhaustion fails closed.

## Verified fast re-registration

Fast re-registration is available only after HBBS finds an existing verified
record with an exact match of:

```text
(peer_id, network_identity_uuid, public_key bytes)
```

The public-key bytes are SHA-256-hashed only for the in-memory burst key. The
optimization does not bypass the global/IP blocker. It permits at most 12
accepted fast re-registrations for that exact identity in a rolling 30-second
window, including exact retries of the current activation; ordinary
registration limits remain in force for unverified or mismatched identities.
An exact retry reuses the current lease/generation instead of minting a new
lease, but still consumes one burst slot. This is an existing-record identity
check, not a new proof-of-private-key protocol.

## Multi-node behavior and Kessoku verification

Each HBBS owns an independent route table, generation sequence, and random
lease. Akari should register the pending activation with every configured HBBS,
retain acknowledgements by server instance, and commit according to its
profile policy only after the required matching ACKs arrive. Deactivation must
be sent back to every issuing HBBS with that node's lease and generation.

Kessoku can call `POST /control/v1/peers:verify` on each instance with the peer
ID, UUID, epoch, activation ID, and one to 16 candidate leases. A node returns
`registered: true` only for its current live lease. Leases and activation IDs
must be treated as secrets and must not appear in URLs, labels, logs, or traces.

## Observability

`GET /control/v1/capabilities` advertises `profile_activation_lease: 1`.
`GET /control/v1/relays` includes `profile_activation` with the fixed limits and
these counters:

| Counter | Interpretation |
| --- | --- |
| `active_leases`, `last_route_generation` | Current local state and generation watermark. |
| `leases_issued`, `leases_reused`, `ready_acks` | Successful lease lifecycle and ACK delivery attempts. |
| `fast_reregistrations`, `renewals`, `route_replacements` | Expected profile-switch activity. |
| `deactivations`, `disconnect_cleanups`, `ttl_expirations` | Normal close, transport cleanup, and crash fallback. |
| `invalid_requests`, `stale_rejections` | Malformed input or generation/lease mismatch. |
| `rate_limited`, `capacity_rejections` | Burst or bounded-registry fail-closed events. |

Alert on sustained increases in stale/invalid/rate/capacity rejections, lease
growth without cleanup, Ready ACKs without client-side commits, or TTL expiry
being used as the routine switch path. Counter changes are aggregate and never
expose peer IDs, UUIDs, activation IDs, public keys, or leases.

HBBR uses no new data message. Existing session/load metrics and disconnect
cleanup remain the signals for detecting rapid-switch pressure at the Relay.

## Release order

1. Deploy patch-v1.3.0 HBBS/HBBR and Control Agent with clients still using the
   legacy path. Confirm capability 1, zero/expected counters, official-client
   registration, and HBBR forwarding.
2. Upgrade Kessoku to understand the capability, per-instance lease set,
   `/peers:verify`, and redaction rules. Keep Profile switching disabled.
3. Canary Akari with matching-ACK commit logic. Exercise Native UDP/TCP, WSS,
   A→B→A, reordered packets, two HBBS nodes, and mixed old/new clients.
4. Expand the Akari cohort only while stale/rate/capacity/TTL counters and HBBR
   disconnect/session pressure remain within the accepted baseline.

## Rollback

First stop Akari from initiating new activation switches and keep its last
committed Profile. Send best-effort `DeactivatePeer` to every issuing HBBS,
then wait at least 45 seconds plus the configured WSS idle/drain interval.
Kessoku may stop calling the new endpoints after active leases reach zero.

The server can then roll back independently: extension fields are in-memory
only and require no database migration. A rolled-back server ignores enhanced
request fields; Akari's matching-ACK rule prevents a false commit. Do not
remove or rotate the stored peer identity/public key merely to clear a lease.
