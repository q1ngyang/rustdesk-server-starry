# Architecture and build

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Architecture-and-Build)

Starry is a reproducible source overlay, not a permanent fork of the complete
RustDesk Server tree. It keeps the official server revision explicit, patches
HBBS plus an additive HBBR public probe and authenticated telemetry channel, while preserving official-client
compatibility and the upstream HBBR byte-forwarding data path.

## Component and traffic boundaries

```text
RustDesk client
  |-- API HTTPS --------------------> optional third-party API
  |-- native 21116 or WSS /ws/id ---> Starry HBBS
  |                                     | selects one Relay
  |-- P2P, native 21117, or /ws/relay -> bundled HBBR (upstream data path)
```

| Component | Starry change | State/role |
| --- | --- | --- |
| HBBS | Yes | Peer registration, rendezvous, Secure TCP negotiation, persistent WSS signalling, Geo evaluation, and Relay allocation. |
| HBBR | Public probe plus authenticated telemetry | Carries bytes through the upstream forwarding path, answers Akari probe v1 without load details, and exposes bounded load plus the exact Starry build only to authenticated HBBS pulls. |
| Control Agent | Separate Starry binary | Linux-only least-privilege management API for one local HBBS; mTLS/service JWT remotely and a bounded loopback protocol locally. |
| `rustdesk-utils` | No | Convenience upstream utility artifact. |
| API | Not included | Login, address book, device/admin data; select and secure independently. |
| Client | Not included | Chooses native or WebSocket and performs P2P/HBBR data exchange. |

The separation matters operationally. API success does not prove an HBBS
handshake. HBBS registration does not prove HBBR reachability. HBBR health does
not prove a two-client desktop session.

## Overlay layout

| Path | Purpose |
| --- | --- |
| `scripts/apply_overlay.py` | Verifies unique upstream source anchors, copies Starry modules/tests/config template, and injects integration points. |
| `overlay/src/starry_config.rs` | Strict schema, defaults, cross-field validation, artifact generation, and atomic configuration state. |
| `overlay/src/geo_relay.rs` and `geo_relay/` | MMDB readers/updater, fact extraction, expression compiler, and ordered Relay selection. |
| `overlay/src/secure_tcp.rs` | Client-compatible native Secure TCP negotiation, authenticated key exchange, and framed encrypted transport. |
| `overlay/src/websocket_signal.rs` and `websocket_signal/` | `/ws/id` admission, persistent registration/session routing, resource limits, effective client IP, and Relay health. |
| `overlay/src/connection_auth.rs` | Ed25519 JWT/JWKS/introspection verification and bounded metrics/cache state. |
| `overlay/src/relay_observer.rs` and `allocation_explain.rs` | Immutable runtime snapshots and the shared pure allocation-decision core. |
| `overlay/src/relay_quality.rs` | Candidate offers, bounded report validation, RTT/jitter/loss/load scoring, prefix caching, and hysteresis. |
| `overlay/src/fast_relay.rs` | Post-selection FastCompat policy gates, Ed25519 authorization signing, source-bound bounded caches, retry reuse, and runtime counters. |
| `overlay/src/profile_activation.rs` | Node-local route leases, one Native/WSS generation authority, exact-current deactivation, disconnect/TTL cleanup, verified burst limiting, and aggregate counters. |
| `overlay/src/local_control.rs` | Bounded loopback `STARRYCTL/1` framing and legacy local-command compatibility. |
| `overlay/src/control_agent.rs` and `control_agent/` | mTLS/RBAC Control API, local client, durable config transactions, audit, history, rollback, and recovery. |
| `overlay/tests/` | Real-process WebSocket/mixed, connection-auth, local-control, and Control Agent/fault integration tests. |
| `config/` | Full schema example plus deployable feature profiles. |
| `docker/Dockerfile` | Runtime image containing the release binaries; default command starts Starry HBBS. |

The application script requires exactly one match for every structural anchor.
If official source changes invalidate an anchor, the build stops. CI applies
the overlay twice; the second pass must be idempotent and the patched tree must
pass `git diff --check`.

## Relay decision pipeline

1. HBBS obtains both effective public client addresses. For WSS, forwarded
   headers are accepted only from configured trusted proxy CIDRs.
2. The requested transport produces an eligibility set:
   native-online, WSS-health-verified, or their intersection for mixed mode.
3. Geo readers extract only the facts required by compiled rules.
4. Rules run in document order; optional symmetry exchanges client A and B.
5. The first eligible Relay in a matching rule's list wins.
6. If no rule selects, official-style round-robin runs over the eligible set.
7. No eligible WSS/mixed Relay produces an empty allocation instead of an
   knowingly incompatible endpoint.

Native relay eligibility remains based on the official online Relay mechanism.
WSS eligibility comes from normal DNS/TCP/TLS, certificate hostname/chain, and
an exact WebSocket Upgrade to `/ws/relay`.

## FastCompat authorization pipeline

1. Akari opts into Relay quality and both endpoint reports are processed under
   the normal Relay decision pipeline.
2. HBBS connection authentication must return exact `allow`; a Secure TCP or
   configured WebSocket signalling path is required.
3. HBBS freezes the final `relay_server` and Relay-quality decision before
   authorization is considered.
4. `fast_relay` binds the session UUID to both endpoint IPs, allocation,
   selected Relay, and active policy generation, then applies cache and
   per-source signing limits.
5. HBBS uses its existing Ed25519 key to sign a short-lived protobuf payload
   that permits FastCompat, explicitly denies FastMediaV1, and carries a
   bitrate ceiling.
6. The target request and controller response receive identical signed bytes.
   Any failed gate emits no grant and keeps the ordinary reliable HBBR path.

The Control Agent exposes only capability/policy and aggregate counters.
Signing keys, tokens, session UUIDs, and signed authorizations remain inside
HBBS/client signalling and never enter Kessoku management records.

## Profile activation pipeline

1. Akari creates a non-zero epoch and a fresh 16-byte activation ID, then
   registers the pending Profile without replacing its locally committed one.
2. HBBS serializes mutations per peer ID, checks the stored ID/UUID/public-key
   tuple, creates a 32-byte node-local lease and a generation shared by Native
   and WSS routing, then commits the route.
3. A successful Ready ACK echoes the epoch/activation ID and carries that
   lease/generation. Akari commits only an exact matching ACK; legacy/default
   responses leave the previous Profile active.
4. Heartbeats and cleanup operate only on the exact current route. Native
   renewal also matches the socket address; WSS cleanup additionally matches
   an internal connection ID, so same-activation retries and delayed packets
   remain harmless after A→B→A.
5. Disconnect cleanup runs immediately where possible. The 45-second route
   TTL is only the crash fallback; bounded records retain enough history to
   reject stale epochs.

Every HBBS has an independent lease registry. Kessoku reconciles each instance
through `/peers:verify`; it never treats a lease as cluster-wide or forwards it
to HBBR. The normative contract is
[Profile Activation Lease v1](../../reference/PROFILE-ACTIVATION-LEASE-v1.md).

## Configuration safety model

- Serde denies unknown fields at every schema level.
- Numeric limits, unique values, URL structure, CIDRs, Origins, and all Relay
  cross-references are validated before activation.
- Missing/empty/invalid Starry configuration keeps official-compatible
  behaviour; a partially parsed policy is never applied.
- Reload is document-atomic: a complete valid document becomes active; an
  empty/invalid one disables Starry and does not retain the previous Starry
  state. Partial policy is never applied.
- MMDB replacement uses a temporary file, structural/readability checks, and
  atomic replacement; the last readable file is retained on failure.
- Management commands are intended only through loopback `21115` inside the
  HBBS namespace.
- WSS registration has frame, queue, session, per-IP, timeout, and rate limits.

These controls reduce configuration and exposure errors; they do not replace
host hardening, secret management, monitoring, backups, or real-client testing.

## Build from an exact official source

The canonical procedure is the GitHub Actions workflow. For local audit or
development on a supported Linux build host:

```sh
git clone https://github.com/q1ngyang/rustdesk-server-starry.git
cd rustdesk-server-starry

git init _upstream
git -C _upstream remote add origin \
  https://github.com/rustdesk/rustdesk-server.git
git -C _upstream fetch --depth 1 origin 1.1.16
git -C _upstream checkout --detach FETCH_HEAD
git -C _upstream submodule update --init --recursive --depth 1

python3 scripts/apply_overlay.py _upstream
python3 scripts/apply_overlay.py _upstream
git -C _upstream diff --check

cargo metadata --manifest-path _upstream/Cargo.toml \
  --format-version 1 >/dev/null
cargo test --manifest-path _upstream/Cargo.toml --locked --lib -j 1
cargo check --manifest-path _upstream/Cargo.toml --locked --bins -j 1
cargo test --manifest-path _upstream/Cargo.toml --locked \
  --test websocket_signal -j 1 -- --nocapture
cargo test --manifest-path _upstream/Cargo.toml --locked \
  --test mixed_relay -j 1 -- --nocapture
cargo build --manifest-path _upstream/Cargo.toml --locked --release --bins
```

Replace `1.1.16` only with a reviewed official release reference. Official
RustDesk Server build prerequisites and Rust toolchain requirements also apply.
An overlay anchor failure is a request to review upstream changes, not an error
to bypass with a broad search-and-replace.

The resulting `hbbs` contains Starry changes. `hbbr` keeps upstream session
forwarding plus the additive public probe/authenticated telemetry surface; `rustdesk-utils` is unchanged.

## Automated release gates

The workflow resolves the official ref and constructs
`<upstream>-patch-v<PATCH_VERSION>`. Before publication it performs:

- Compose static validation;
- exact shallow upstream checkout and recursive submodules;
- twice-applied overlay/idempotency and dependency-lock checks;
- Rust formatting, all library tests, and all server-binary checks;
- real-process WSS registration and cross-transport signalling tests;
- active Relay probe/authenticated-telemetry checks plus mixed WebSocket/native traffic through
  the bundled HBBR's upstream byte path;
- static Linux `amd64` builds;
- installation and command-level runtime checks for amd64 Debian packages
  under the digest-pinned Debian test image;
- a `linux/amd64` container smoke test; and
- assembly of the exact downloadable candidate, including source/final-tree
  SPDX SBOMs, deterministic archives, build inputs, and verified checksums.

Only the separately approved publication job has write permissions. After all
candidate gates pass, it creates or verifies an annotated release tag bound to
the exact source commit; an existing tag is accepted only when it resolves to
that commit. It then pushes the `linux/amd64` image with OCI provenance/SBOM,
records the image-index and platform-manifest digests together with the
OpenAPI, config schema/UI schema, frozen Relay Quality protocol, and Relay
telemetry schema digests in `STARRY-RELEASE-SUMMARY.json`, and includes that
summary in the checksummed GitHub Release. The final downloadable subjects and
SBOM are covered by GitHub/Sigstore artifact attestations and portable bundles.
The workflow never moves or replaces the annotated source tag.

ARM remains best-effort source compatibility, and the Windows build is an
experimental non-blocking check. Neither enters the patch-v1.3.0 candidate.

A successful candidate build does not itself change a Release, attestation
store, or GHCR package. Deployment acceptance remains the operator's
responsibility.

## Image and artifact model

The GHCR image contains `hbbs`, `hbbr`, and `rustdesk-utils` for convenience.
Its default command runs:

```text
hbbs --starry-config=/root/starry/config.yaml
```

The recommended Compose files use one pinned Starry image tag for both HBBS
and HBBR. This prevents independently updated images from drifting while the
command boundary still makes the modification scope explicit: `hbbs` contains
the selection overlay and bundled `hbbr` preserves upstream forwarding while
adding only the public quality-probe and authenticated-telemetry control surface.

Release checksums cover downloadable assets. Portable Sigstore bundles and
GitHub artifact attestations bind the downloadable subjects to their build and
SBOM assertions. Image digests, OCI provenance, and OCI SBOM describe the
container supply chain. `STARRY-RELEASE-SUMMARY.json` is the machine-readable
bridge between those image digests and the exact OpenAPI/schema inputs used by
Kessoku; verify it through `SHA256SUMS` before pinning it.

## Version maintenance checklist

When changing either upstream or patch version:

1. review upstream source and protocol changes;
2. update `PATCH_VERSION` only for a Starry feature/fix release;
3. re-run the overlay against the exact candidate source;
4. update both release-note languages, changelog, image examples, and upgrade
   notes;
5. verify all published examples and relative links;
6. review generated Release/GHCR descriptions; and
7. obtain explicit publication approval after the final documentation diff is
   reviewed.

## Legal and provenance notice

This is an unofficial community project and is not affiliated with RustDesk,
MaxMind, any MMDB provider, or any AI service provider. MMDB files are not
included. Operators must select lawful data sources and follow their licences.
Parts of the code and documentation were generated or revised with AI
assistance; they receive no separate warranty and remain under the repository's
licensing terms.
