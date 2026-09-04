# patch-v1.3.2 preview notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.3.2.zh-CN.md)

patch-v1.3.2 is a backward-compatible active-session-renewal patch based
exactly on the published tag `1.1.16-patch-v1.3.1` at commit
`1b8080bf074e3236cf9a3c0dfae2bdf16832249e`. It changes only Starry. Relay
Quality v1, schema v5, AKR1 kinds 1–5, the reliable HBBR data plane, official
client behavior, and the patch-v1.3.1 fields 1–12 remain frozen.

**Release state: PREVIEW_APPROVED after the repository gates documented
below.** This permits a prerelease and rolling `preview` image only. It does
not permit `stable` or `latest`; those channels remain blocked on a real
Akari controller/target long-session run, cross-network NAT/UDP fault soak,
and immutable hosted-artifact provenance.

## Frozen renewal contract

The protocol was frozen before the implementation in commits
`1844654a272d70112bbbc7774414320c98aa3b99` and
`f65981fbf8c77dd93cfd026422e528128aa13d1c`. The canonical manifest is
[`contracts/patch-v1.3.2/CONTRACT-RELEASE-SUMMARY.json`](../../contracts/patch-v1.3.2/CONTRACT-RELEASE-SUMMARY.json),
whose SHA-256 is
`0158980c7c9b3e3d50cda29d5737fefc52a3324da492034542f91d8ac5c55784`.
Its per-file hashes freeze:

- Rendezvous tags 106/107 and Fast Relay authorization fields 13–16;
- the renewal lifecycle, monotonicity, binding, replay, admission, privacy,
  and compatibility rules plus the semantic fixture;
- authenticated Relay telemetry schema/fixture v3; and
- Control OpenAPI 1.1 and the typed `capabilities`/`relays` fixtures.

Schema v5 is unchanged. Its only machine expression remains
`capabilities.config_schema = 5`. Renewal is negotiated independently as
`capabilities.fast_media_relay_renewal = 1`, and per Relay as
`relays[].capabilities.fast_media_relay_renewal = 1`, only when HBBS has fresh,
authenticated telemetry v3 with `fast_media.renewal_protocol = 1`. Version
strings are never capability signals.

## What changed

- HBBS retains a bounded, role-specific grant chain for its own selected
  Relay. It signs both renewed grants with the existing Starry Ed25519 key and
  returns them only on the authenticated encrypted signalling route that
  carried the request. Clients and HBBR cannot sign or extend grants.
- WSS renewal remains exact-route bound. Because the native initial Relay
  response consumes its one-shot writer, a later authenticated Secure TCP
  request may change only the controller source port while preserving its IP
  and every session, Relay, allocation, protocol, cap, sequence, and grant
  digest binding. Plaintext TCP and a changed IP fail closed.
- Exact retries return the byte-identical cached response. Conflicting reuse,
  skipped/old sequences, changed session IDs, mismatched role hashes, raised
  bitrate, changed Relay/protocol/datagram bound, and expired new grants are
  rejected.
- HBBR accepts a renewed authorization only through a fresh AKR1 Cookie/Bind.
  Both roles may differ by one sequence only during a bounded transition.
  Renewal and rebind preserve the 2,048-packet AKF1 replay window and traffic
  history. A valid unexpired renewed pair can recover after an HBBR restart.
- Fully bound renewable allocations no longer disappear at bootstrap expiry
  or the old 300-second creation ceiling. Half-bind and idle TTLs, bounded
  cleanup, capacity/admission limits, expired-role denial, a bounded
  post-expiry recovery period, and an absolute 12-hour default/24-hour maximum
  session lifetime remain enforced.
- HBBS and HBBR reserve the wire-rate budget before accepting a role. The
  signed encoded-source cap can only decrease; its wire allowance remains
  `ceil(source × 1.45)`. Same-NAT roles accumulate against the same bounded
  IP budget.
- Any renewal, UDP, grant, bind, admission, restart, or rate-limit failure
  affects only FastMedia. The reliable desktop/HBBR stream stays connected so
  Akari can fall back and later re-enter the same FastMedia session.

## Exact Akari implementation checklist

Use the canonical protobuf in
[`contracts/fast-media-renewal/v1/rendezvous-extension.proto`](../../contracts/fast-media-renewal/v1/rendezvous-extension.proto)
and the state machine in
[`FAST-MEDIA-RENEWAL-v1.md`](../reference/FAST-MEDIA-RENEWAL-v1.md).

1. Keep the reliable Relay session established throughout FastMedia. Enable
   renewal only when both bootstrap role grants have field 13
   `fast_media_relay_renewal = 1`; do not infer it from a Starry version.
2. Parse authorization fields 14 `relay_session_id`, 15
   `renewal_sequence`, and 16 `previous_authorization_sha256`. Bootstrap is
   session ID/sequence zero and an empty previous hash.
3. The controller chooses one non-zero AKR1 session ID and delivers it to the
   target through the already authenticated, end-to-end encrypted reliable
   session. Both endpoints must bind that exact value.
4. At `renew_after`, the controller sends `FastMediaRenewalRequest` (oneof
   tag 106) on its authenticated WSS route, or a new authenticated Secure TCP
   connection from the same IP. Populate protocol version 1, exact UUID,
   allocation, session ID, current sequence, SHA-256 of each complete current
   combined grant, a fresh random 16-byte request ID, existing connection
   token, exact HBBS-selected Relay/protocol/datagram/bitrate values, and
   requester role 1.
5. Retry only the exact serialized logical request with the same request ID
   and bounded backoff. Treat response tag 107 statuses numerically:
   `1 OK`, `2 DISABLED`, `3 UNAUTHENTICATED`, `4 NOT_FOUND`,
   `5 BINDING_MISMATCH`, `6 EXPIRED`, `7 TOO_EARLY`, `8 RATE_LIMITED`,
   `9 UNAVAILABLE`, and `10 INVALID`.
6. On `OK`, verify all echoed bindings, sequence `current + 1`, increased
   expiry, and a cap no higher than the current cap. Install only the
   controller grant locally. Deliver only the target grant over the existing
   authenticated encrypted desktop channel; never via Kessoku, telemetry, or
   Control API.
7. Each role obtains a fresh source-bound cookie and sends AKR1 Bind with its
   new grant. Tolerate a role sequence difference of one only for the bounded
   transition window. Do not reset the AKF1 sequence/replay state on renewal
   or source migration.
8. Start reliable-media fallback no later than `fallback_before` (ten seconds
   before expiry) unless both new role binds are usable. Preserve the last
   frame, control/input, and reliable session. Bounded retries may later
   re-enter FastMedia automatically with the same session ID.
9. Never select a Relay locally, self-sign, accept an expired grant, raise a
   cap, change any pinned binding, or treat allocation ID alone as authority.

No Akari implementation may be declared complete without controller and
target coverage for exact retry, conflicting replay, delayed second-role
install, source-port migration, listener restart, reliable fallback, and
same-session automatic re-entry.

## Compatibility matrix

| Deployment | Behavior |
| --- | --- |
| Official client or official/legacy HBBR | Ordinary P2P/LAN/reliable Relay is unchanged; unknown additions are ignored. |
| Old Akari + patch-v1.3.2 | Uses its bootstrap grant and safely falls back at the original expiry. |
| New Akari + patch-v1.3.1 HBBR | No authenticated renewal capability; retains v1.3.1 90/300-second behavior and fallback. |
| patch-v1.3.2 HBBR + older HBBS | Renewal is not advertised through HBBS; bootstrap behavior remains. |
| New Akari + patch-v1.3.2 HBBS/HBBR | Renewal is eligible only after final server Relay selection, auth allow, and fresh matching telemetry v3. |
| Mixed official/new Akari | Official endpoint ignores private messages; the server-selected reliable Relay stays authoritative. |

All schema-v5 Fast switches remain independently default-off. A telemetry-v2
Relay remains eligible for v1.3.1 bootstrap FastMedia but is not renewable.

## Telemetry, Control, and privacy

Authenticated telemetry v3 adds bounded renewal, replay, reservation,
admission, transition/recovery, remaining-TTL, and approaching-expiry
aggregates. Control API 1.1 exposes their sanitized fixed fields. Neither path
contains full client addresses, session/allocation/request IDs, nonce, token,
stage token, grant, raw report, or media content.

`process_instance_id` is used inside Starry to detect HBBR restarts. Kessoku
must discard it at ingestion and must not forward, persist, index, log, or
display it.

Alert on sustained stale/unhealthy telemetry, an enabled-but-unhealthy UDP
listener, rising listener/auth/admission/rate/expired rejection counters,
renewals approaching expiry, low minimum remaining TTL, or reservations near
their limits. Bind/rebind/renewal success, replay classes, packets/bytes, and
role-transition counts are diagnostic unless correlated with reliable
fallback or user-visible failure.

## Kessoku conclusion

Kessoku is not in the grant delivery, signing, media, or renewal path.
Kessoku v3.0.8 can safely ignore the additive Control fields and operate the
existing schema-v5 configuration, but it has no renewal visibility. Kessoku
v3.0.9 is required only if it must consume/display the new typed aggregate
capability and telemetry fields. It must pin a clean, pushed contract/source
candidate—not a dirty worktree—and enforce the `process_instance_id` discard
rule above.

## Upgrade and rollback

Roll HBBR first, HBBS second, Control Agent third, and renewal-capable Akari
last. Keep FastMedia disabled until authenticated telemetry v3 is fresh and
the ordinary Native/WSS/mixed Relay regression passes. Canary a bounded Relay
and client cohort before wider use.

Rollback does not require a schema conversion because schema v5 is unchanged.
Stop issuing renewals, let clients fall back to the reliable stream, disable
FastMedia with an activation ACK, and drain active allocations/grants. Roll
HBBS back to `1.1.16-patch-v1.3.1`, then HBBR. Older binaries ignore fields
13–16 and telemetry-v3 additions; enrollment, identity, YAML/PEM/JWKS, ordinary
Relay/WSS, and persisted pairing state must be preserved. Re-upgrading requires
fresh telemetry v3 before renewal is eligible again.

## Verification and remaining gates

The reviewed implementation candidate is
`e44f6a0380914454cc543ebb6cdb031f5b3e08f9`. Its machine-readable
[`verification manifest`](VERIFICATION-patch-v1.3.2.json) records the exact
commands, counts, baseline revisions, and apply-twice hashes. The source
candidate includes unit, contract, controlled-clock, and real
HBBS/HBBR subprocess coverage for dual-role bootstrap and renewal, exact retry,
conflict/expiry/binding/cap failures, delayed roles, replay persistence,
same-IP native source-port rotation, exact WSS routing, plaintext rejection,
HBBR restart recovery, AKF1 forwarding, admission/rate limits, and survival of
the reliable stream. It also retains the full protocol, Control, mixed Relay,
connection-authentication, local-control, WebSocket, and 1,000-registration
release gates. Exact command results are recorded in the reviewed source
candidate verification manifest; the publishing CI repeats them. No immutable
tag or hosted image digest is claimed by this source-only evidence.

The following block only `stable`/`latest`, not a source-controlled preview:

- immutable real Akari controller and target builds with dependency lock and
  provenance, running a normal-length renewed desktop session;
- traceable devices/runners across real NAT, UDP block, AP/source migration,
  listener restart, burst loss, sustained cap, fallback, and automatic re-entry;
- an immutable tag, source commit, image-index and linux/amd64 digests, and
  hosted release evidence for one exact reviewed commit.
