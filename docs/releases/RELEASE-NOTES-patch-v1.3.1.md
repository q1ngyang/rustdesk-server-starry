# patch-v1.3.1 preview notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.3.1.zh-CN.md)

patch-v1.3.1 is developed from the frozen patch-v1.3.0 baseline commit
`abf1dbcfdf7c4384f8c7ac34724089932a1bc58c`. It adds the Starry half of Akari
FastCompat/FastMediaV1 Relay support and Starry Pairing v1 while preserving
official clients, old Akari, manual deployments, and the frozen Relay Quality
v1 contract.

**Release state: PREVIEW_APPROVED.** The wire and Control contract is frozen at commit
`6f5a31008ab7761d8557c8cf9fefcb5be11c49e6`, whose
`CONTRACT-RELEASE-SUMMARY.json` SHA-256 is
`67cc28287ed8c6fedfc37b88c6b0ecbc95a734a4644a34bfbb2d85e6d801df67`.
The source candidate has passed a protocol-level
Akari↔HBBS↔HBBR dual-role authorization/bind, encrypted forwarding, reliable
fallback, and same-session automatic re-entry harness. This is sufficient for
an opt-in preview whose Fast switches remain off by default. It is not stable
runtime approval: the real two-client GUI/signalling, device/NAT/fault matrix,
and production PKI gates have not passed for one exact reviewed commit.

## Fast Relay and FastMedia

- HBBS now signs its own final Relay even when Relay Quality is disabled, has
  fewer than two compatible candidates, times out, or uses legacy fallback.
  A completed Relay Quality v1 decision remains authoritative, and ordinary
  `relay_server` always equals the signed `relay_server`.
- `FastRelayAuthorization.version = 1` retains fields 1–6 and appends tags
  7–12: UDP protocol, selected Relay, UDP port, fresh 16-byte allocation ID,
  datagram bound, and endpoint role. Controller is role 1 and target is role 2.
- FastMedia sessions receive two different Ed25519 combined grants; an old
  six-field FastCompat grant remains compatible. Both feature switches default
  to false and failures retain the reliable HBBR stream.
- HBBR implements the 32-byte AKR1 envelope and Hello/Cookie/Bind/Bound/Media
  state machine. It uses a source-bound stateless cookie, verifies role grants,
  pins allocation/session/Relay/source tuples, waits for both roles, validates
  the clear AKF1 preamble, strips AKR1, and forwards encrypted AKF1 unchanged.
- Same-role migration requires a new cookie and atomically revokes the old
  tuple. Grant size, datagram size, expiry, half-bind/idle/absolute lifetime,
  allocation count, cleanup work, replay window, rebinds, and per-role/per-IP/
  global traffic are bounded.
- The signed bitrate is the encoded-source ceiling. HBBR permits at most
  `ceil(source × 1.45)` wire Kbit/s with a role burst bounded by
  `max(256 KiB, 50 ms wire allowance)`.

## Relay Quality v1 correction

The frozen protobuf, tags, scoring, staged strategy, telemetry interpretation,
hysteresis, reason codes, privacy, and digest are unchanged. Only the native
initial-response route guard is corrected: `PunchHoleSent`/`LocalAddr` may use
a new target source port when the original controller route, target IP and ID,
allocation, stage/token, target role, candidate set, and generation all match.
Top-level reports and controller routes remain exact; one identical duplicate
is idempotent and a conflicting duplicate is rejected.

## Schema v5, telemetry, and Control API

Schema v5 adds independent, default-off `fast_compat_enabled` and
`fast_media_v1_enabled`, `relay_max_datagram`, and a per-Relay
`fast_media_udp_port`. FastMedia authorization requires fresh authenticated
telemetry schema 2, explicit `fast_media_relay_udp = 1`, and a healthy UDP
listener. Public client probes do not expose this load/runtime data.

Authenticated HBBR telemetry adds capability, UDP health, active allocations
and streams, and bounded cookie/bind/grant/role/session/allocation/rebind/
forward/drop/rate/replay/expiry/listener counters. Control API exposes only
typed per-Relay status and bounded aggregates. It never exposes full client
addresses, UUIDs, allocation IDs, nonces, stage tokens, grants, secrets, or
media.

## Starry Pairing v1

- `starry-control-agent pair`, `adopt`, and `rotate` consume a short-lived,
  single-use `SP1` code only from stdin or a mode-0600 file. The Agent generates
  its private key and CSR locally, validates Broker origin/SPKI pin, response
  bindings, key/certificate match, CA signature, and certificate lifetime, and
  never overwrites an unrelated existing identity. An explicit
  `--tls-server-name` is placed in the CSR as a validated DNS SAN; interrupted
  first pairing reuses its durable pending instance UUID, and rotation
  preserves the validated installed Agent-v1 runtime settings.
- Relay enrollment is authorized by the existing mTLS/JWT Control Agent API.
  Kessoku may broker claim delivery but cannot choose Relay endpoints, pools,
  secrets, or configuration. Prepare/complete/health-activate/revoke/list/get
  operations are bounded and idempotent. Health activation additionally binds
  a succeeded config operation ACK to the exact generation and a freshly read
  HBBS inventory snapshot before the enrollment becomes active. Revocation
  retires only the exact current credentials, and a later same-node enrollment
  may clean up only a matching revoked or expired predecessor.
- `starry-relayctl enroll` generates the Relay node key locally without
  changing upstream `hbbr` CLI. Each Relay gets a separate telemetry secret,
  certificate, approved runtime configuration, and a non-secret
  `relay-compat.env` that points to the secret file.
- Manual Agent YAML/mTLS/JWT and manual HBBR public-key/secret-file operation
  remain supported. Established data planes do not depend on the Broker.

## Persistent deployment boundary

All container Control identity/state/shared-token/generated config,
per-Relay material, and compatibility snapshots live below
`STARRY_PERSIST_ROOT`. Relay enrollment lives below `RELAY_DATA_DIR`.
Pairing/startup rejects container overlay/tmpfs state, unsafe path types or
permissions, absent explicit mounts, and Relay host-identity mismatch. The
image has no anonymous `/root` volume.

`pull`, `force-recreate`, and `down`/`up` retain identities when the same host
roots are mounted. `down -v`, mounting a different relative directory, and
concurrent identity clones are fail-closed operator errors. Native/DEB layouts
use `/etc/rustdesk-server-starry` and `/var/lib/rustdesk-server-starry`; normal
package upgrade/downgrade does not overwrite identity.
The Relay-only Compose profile defaults enrollment enforcement to `1`; the
preserved manual public-key mode requires an explicit `0` and must not be used
to restart a previously enrolled Relay with missing state.
The compatibility-file parser splits each allowlisted assignment at its first
delimiter so standard Base64 padding in the public RustDesk `KEY` is retained.

## Upgrade and rollback

Upgrade HBBR first, then HBBS, Control Agent, and finally compatible Akari.
Keep both Fast switches disabled until authenticated schema-2 telemetry and
reliable-session regression tests pass. Canary FastCompat before FastMedia.

For v1.3.1→v1.3.0, first disable FastMedia and use
`starry-control-agent config downgrade --to-schema 4 --preview`. The command
queries the local Agent for active allocations, authorizations, streams, and
latest grant expiry; an audited `--runtime-state` file is an explicit offline
override. Export is refused until all are drained and every Agent/Relay
certificate has at least ninety days remaining. The output removes only v5
fields and never overwrites its destination.

patch-v1.3.0 can read pairing-generated Agent v1 YAML/PEM/JWKS. Enrolled HBBR
uses `relay-compat.env` for its public `KEY` and existing telemetry secret-file
path, retaining ordinary Native/WSS Relay and telemetry. The old version
ignores rather than deletes enrollment/FastMedia state; upgrade back to v1.3.1
reuses identity and requires fresh UDP health.

## Compatibility matrix

| Client/HBBR combination | Result |
| --- | --- |
| Official clients + official/legacy HBBR | Ordinary P2P/LAN/Relay is unchanged; no Fast grant or UDP candidate. |
| Old Akari + patch-v1.3.1 HBBR | Six-field FastCompat remains accepted; unknown fields are ignored; reliable Relay remains. |
| New Akari + legacy HBBR | HBBS may issue FastCompat for its final Relay but never FastMedia; UDP capability is null/fail-closed. |
| New Akari + healthy patch-v1.3.1 HBBR | Role grants and AKR1 are eligible when policy/authentication gates pass; reliable Relay remains connected. |
| Mixed official/new Akari | Official endpoint ignores tag 64; the ordinary HBBS-selected Relay remains interoperable. |
| v1.3.1 enrollment + v1.3.0 runtime | Agent v1 credentials and ordinary Relay/telemetry survive; pairing automation and FastMedia are ignored. |

## Frozen contract candidate

Relay Quality v1 remains the already-frozen inherited contract. The unique
patch-v1.3.1 `CONTRACT-RELEASE-SUMMARY.json` freezes Control OpenAPI, config
schema v5/UI schema, `capabilities`/`relays`/`status`, Relay enrollment, SP1,
twelve-field Fast Relay authorization, AKR1, telemetry v2 schema/fixture, and
the downgrade drain-state schema by per-file SHA-256. Its source binding is the
Git commit containing that summary. Schema support is expressed only as
`capabilities.config_schema = 5`, accompanied by supported/active versions and
the schema digest. Kessoku v3.0.8 may pin the exact pushed contract-candidate
commit, never a dirty worktree. `PREVIEW_APPROVED` authorizes only a GitHub
prerelease and the rolling `preview` image; it is not stable runtime approval.

## Verification status

The repository includes unit and integration coverage for schema defaults and
gates, server-selected fallback grants, role swapping/tampering/expiry,
cookies, dual bind, AKF1 forwarding and replay, source-port rebind, rate and
lifecycle limits, authenticated telemetry and Control JSON, SP1 expiry/replay/
purpose/key/digest/pin checks, interrupted install recovery, enrollment
idempotency, persistence-mount checks, and no-side-effect downgrade export. A
real HBBR subprocess test exercises both roles and proves its reliable TCP path
survives UDP work.

Current working-tree validation passed all 141 library tests, 11 HBBR binary
tests, the 12 protocol-contract tests, the directed Control Agent, connection
authentication, local-control, mixed-Relay, WebSocket, and real-HBBR
integration suites, and the explicit 1,000-WebSocket release gate. The
cross-repository Akari protocol harness kept the reliable TCP stream alive,
fell back after a UDP probe timeout, and re-entered FastMedia in the same
session. This proves the protocol state transition, not a GUI remote-control
session or a real-device/network result. The CI-owned rustfmt set, `cargo check
--all-targets`, `cargo clippy --all-targets`, native and amd64-musl release
builds, local image command/persistence smoke, reproducible four-package DEB
build, pinned-Debian install smoke, and an HBBS
v1.3.1→v1.3.0→v1.3.1 identity round trip also passed. Applying the overlay
twice to clean official commits produced the same full-file digest
`6cc4aea565e8968973715280d7739f85db67531726974345b3ab1029a286ab85`.
The locked dependency audit reports zero vulnerabilities, zero unsound
advisories, and zero yanked crates after moving the yanked
`chacha20 0.10.1` lock entry to non-yanked `0.10.2`; the disclosed upstream
`sodiumoxide 0.2.7` unmaintained warning remains.

An isolated, non-publishing exact-working-state staging exercise against
Kessoku v3.0.8 used one persistent `KESSOKU_DATA_DIR` and passed SP1 Control
certificate rotation, mTLS/JWT inventory after restart, health-gated Relay
re-enrollment, force-recreate persistence, stopped backup/restore, and exact
high-risk rollback confirmation. A coordinated
v3.0.8/v1.3.1 → v3.0.7/v1.3.0 schema-v4 → v3.0.8/v1.3.1 round trip preserved
the independent registry, generation, HBBS identity, generated Agent-v1
identity, ordinary Native/WSS Relay, and fresh telemetry. Direct Kessoku
v3.0.7 → Starry v1.3.1 inventory was correctly rejected because the former
freezes config schema ≤4 and telemetry schema 1. The restored Kessoku v3.0.8
inventory deliberately dropped process/allocation/session UUIDs, addresses,
tokens, nonces, grants, private keys, and media fields. These images are
diagnostic working-tree artifacts, not immutable release artifacts; every
applicable operation must be repeated for the final clean commit.

The following remain stable-release blockers until recorded for one exact
candidate; findings become a later patch preview and do not revoke this
immutable preview:

- real two-client Akari GUI remote-control sessions through HBBS/HBBR across
  Native, WSS, and mixed signalling, including observable reliable fallback
  and automatic re-entry without session loss;
- real-device UDP-block, HBBR-restart, NAT/AP migration, 300–1200 ms path
  migration, shaped loss/overload, long video/thermal, and reconnect soak;
- production-PKI certificate rotation, enrolled-Relay rotation before a
  90-day rollback window, and multi-host migration/clone/down-volume drills;
- an immutable, clean Akari candidate plus hosted clean-commit CI/security
  review, history secret scans, SBOM/signing/provenance/attestations,
  multi-architecture artifact reproducibility, and the full Docker/DEB/native
  pairing and Relay cross-upgrade matrix.
