# patch-v1.2.0 release notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.2.0.zh-CN.md)

patch-v1.2.0 prepares strict connection authentication, observable Relay
allocation, and a least-privilege management plane while preserving Starry's
overlay model and the official RustDesk wire protocol.

These are the release notes for the full `1.1.16-patch-v1.2.0` artifact tag.

The reviewed release-preparation change sets `RELEASE_STATUS` to `APPROVED`.
Publication still fails closed unless every source, security, test, package,
image, and release-candidate job succeeds for this exact commit.

## Component and platform scope

- The overlay still changes HBBS only. HBBR remains unmodified.
- No account, address-book, or device API is included.
- `starry-control-agent` is a separate optional management binary. Its remote
  API never proxies the HBBS `21115` protocol and exposes no shell, arbitrary
  file, URL, process, Docker socket, or generic command operation.
- Writable Control Agent transactions and all promised v1.2.0 artifacts are
  supported on Linux amd64. The release contains a `linux/amd64` image, Linux
  x86_64 binaries/tar, and amd64 DEBs. ARM remains best-effort source
  compatibility; Windows is an experimental non-blocking build. Neither ARM
  nor Windows enters the v1.2.0 candidate.

## Recommended Docker deployment

Docker Compose on a Linux amd64 host remains the recommended deployment path.
The published image contains Starry HBBS, an unmodified convenience HBBR,
`rustdesk-utils`, and the optional Control Agent; the recommended single-host
example deliberately runs HBBR from the matching official RustDesk Server
image so the component boundary remains visible.

- [GHCR image page](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)
- [Container image guide](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/CONTAINER.md)
- [Recommended Docker deployment guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment)
- [Single-host Compose example](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/examples/compose.yaml)
- [Control Agent sidecar example](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/examples/control-agent/compose.yaml)
- [Multi-node deployment guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment)

After publication, pull the immutable release tag rather than relying on
`latest`:

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0
```

## Configuration lifecycle

- Schema v3 adds `connection_auth`; schema v1/v2 remain accepted with
  authentication off.
- Runtime state now records generation, exact source digest, effective digest,
  activation time, and subsystem acknowledgements.
- A missing, empty, or invalid first configuration keeps upstream-compatible
  behavior. A rejected later reload retains the active last-known-good
  generation and reports the error.
- Candidate parsing/validation is separated from activation. Configuration is
  not reported as active until required subsystems acknowledge the same
  generation.

## Connection authentication

- One shared verifier runs after bounded frame/protobuf parsing and before
  target lookup, punch recording, Relay selection, or delivery.
- It covers controller `PunchHoleRequest` and direct `RequestRelay` on native
  TCP, negotiated Secure TCP, and `/ws/id` WSS.
- UDP initiation remains unsupported and receives no allocation path.
- Native controlled-endpoint registration and heartbeats in official 1.1.16
  still use the upstream UDP path. That path does not carry the controller
  requests above and cannot bypass controller authentication. Use WSS
  registration when controlled-endpoint UDP must be disabled.
- Only access JWTs with `alg=EdDSA`, `typ=at+jwt`, and an explicit `kid`
  selecting a public Ed25519 JWK are accepted. Issuer, audience, token use,
  complete scope value, numeric `user_id`, decimal `sub` binding, positive
  `auth_version`, UUID `jti`, `iat`, `nbf`, `exp`, token size, and signature
  are mandatory. These rules match Kessoku's issued wire representation and
  the checked-in byte fixtures.
- JWKS rotation is atomic and last-known-good. Remote-cache freshness is stored
  in a digest-bound sidecar so restart or reload cannot reset key age. Missing,
  mismatched, or stale metadata, invalid refreshes, and configured
  introspection failures cannot fail open.
- Any configured remote JWKS or introspection endpoint requires HTTPS plus an
  explicit CA, client certificate, client key, and exact DNS server name.
  These internal clients require TLS 1.3 and disable system roots and
  redirects. Local
  JWT failure never calls introspection; an introspection request uses
  Kessoku's strict token-only DTO and the returned subject must match the
  locally verified JWT.
- `audit` records would-allow/would-deny without changing the connection;
  `enforce` uses stable, target-independent existing protobuf denial fields.
- `--must-login` is a deployment enforce floor that config reload and the
  remote management plane cannot lower.

The normative profile and byte fixtures live under [`contracts/auth/v1`](contracts/auth/v1).

## Relay visibility and simulation

- HBBS publishes an immutable Relay/config/health snapshot to the local
  control layer.
- Relay probe or readiness changes advance the health snapshot identity, so
  inventory, production allocation, and simulation on that snapshot can be
  correlated accurately.
- Allocation simulation shares the production decision core but does not
  advance rotation, mutate health/config state, create a Relay UUID, deliver a
  peer message, or increment production allocation counters.
- Responses include generation/snapshot identity, normalized endpoint facts,
  ordered rule/Relay decisions, the selected result, and warnings.

## Control Agent v1

- Remote access requires both a client certificate chaining to the configured
  CA with an exact allowed URI SAN and an independent short-lived EdDSA service
  JWT bound to the Agent instance and requested scope.
- The fixed API provides capabilities, status, Relay inventory, allocation
  simulation, config schema/state/validation, configuration transactions,
  operation lookup, history, rollback, and audited runtime reload.
- Every known HTTP action is mapped to an exact scope and verifies the mTLS/JWT
  principal before body allocation. Unknown actions remain 404.
- The loopback HBBS bridge accepts only bounded `STARRYCTL/1` frames carrying a
  constant-time-checked secret from an absolute, regular, mode-0600 token file.
  The legacy text-command dispatcher is not reachable at runtime.
- `config.write_enabled` defaults to `false`. In that profile the Agent omits
  write capabilities and returns 404 for plan/apply/rollback/reload after
  authentication.
- Apply uses a plan bound to instance, caller, exact-byte ETag, runtime
  generation, candidate digest, and expiry. `If-Match` and an idempotency key
  are mandatory for mutations.
- Resident plans are bounded by count and aggregate bytes; idempotent replays
  are bound to the Agent instance and the authenticated caller identity.
- Intent, operation, idempotency result, revision, recovery material, and
  redacted audit data are durable. The raw JWT and raw idempotency key are not
  persisted.
- Candidate publication uses temporary-file create/write/fsync, atomic rename,
  directory fsync, and synchronous HBBS activation acknowledgement. Failure
  restores the original bytes and runtime; unresolved recovery enters
  `manual_intervention_required` and blocks later writes.
- A write-enabled Agent validates at startup that an existing managed config
  is a single-link regular file owned by the Agent's effective UID and primary
  GID, so an ownership-preserving atomic replacement cannot fail late.

See [`contracts/control/v1/openapi.yaml`](contracts/control/v1/openapi.yaml) and
the [Control Agent guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent).

## Upgrade and rollout

1. Back up the HBBS data, exact config bytes, identity key, and prior image or
   package. Do not start the Agent yet.
2. Upgrade HBBS while retaining the existing schema v1/v2 config and verify
   native, Secure TCP, WSS, and mixed paths that the deployment already uses.
3. Move to schema v3 with `connection_auth.mode: off`; reload and verify the
   activation acknowledgement and generation.
4. Deploy the Agent in read-only mode on a private management path and verify
   mTLS, service-JWT scopes, status, Relay inventory, and simulation.
5. Test write transactions and rollback in staging before setting
   `write_enabled: true` anywhere else.
6. Deploy compatible client JWT issuance, JWKS, and mTLS introspection. Run
   `audit` for a complete business cycle and reconcile every would-deny reason.
7. Canary `enforce` on one instance or user cohort; then expand only after real
   client P2P/Relay/WSS/mixed and dependency-failure tests pass.

## Rollback

- Authentication rollback is a local, controlled schema v3 change from
  `enforce` to `audit`, followed by an acknowledged reload. The remote Agent
  intentionally has no special "disable authentication" endpoint.
- Stop the Agent or return it to read-only without stopping HBBS/HBBR; data
  plane behavior continues from the last active HBBS config.
- Before rolling HBBS back to patch-v1.1.0, restore the pre-upgrade schema v2
  bytes. patch-v1.1.0 does not understand schema v3.
- If an operation reports `manual_intervention_required`, do not retry writes.
  reconcile disk bytes and HBBS runtime digest from the durable recovery/audit
  records first.

## Pre-release validation status

As of 2026-08-20, the isolated local release candidate has completed:

- 76 library tests under Rust `1.97.1`, every binary check, and every protocol
  and integration target, including deterministic token/protobuf mutation
  corpora, both authentication requests on native/Secure TCP/WSS, local
  control, Control Agent atomic transactions, mixed HBBR, and 1,000 registered
  idle WSS sessions;
- an `audit`-to-`enforce` matrix with the unchanged official RustDesk 1.4.9
  Linux client binary
  `sha256:7244ba47d14225a7aa1ae2d6802925b7680c3f2dd16e79c28a2f6dd4066e3687`:
  an invalid audit token still reached a desktop and incremented would-deny;
  valid native TCP and WSS enforce sessions passed; missing/expired tokens,
  logout, disabled user, pre-password-reset token, and a retired key were
  denied; and the post-reset token passed;
- both mixed Relay directions with the same official client under enforce:
  WSS controller to native controlled endpoint and native controller to WSS
  controlled endpoint both paired one HBBR UUID and displayed the remote
  desktop. Native controlled-endpoint registration/heartbeats remain UDP in
  official 1.1.16; controller initiation is authenticated over TCP/Secure TCP,
  while UDP initiation produced no response or allocation;
- Kessoku mTLS JWKS/introspection E2E for current/previous overlap, new key,
  retired old key, logout/disable/password-reset, and fail-closed behavior.
  After the HTTP idle-pool correction, twelve consecutive 30-second JWKS
  refreshes over six minutes succeeded and continuously advanced key age;
- a local seven-container topology of one HBBS and six independent HBBRs. On
  first-Relay failure and recovery the snapshot IDs were `health-14`,
  `health-50`, and `health-80`; simulation selected relay1, relay2, then relay1
  without mutating production rotation or state;
- checksum-backed online SQLite backup plus Starry identity/config/token and
  Kessoku config/key restore into a fresh network. Database integrity, user and
  auth version, token introspection, HBBS identity, verifier readiness, and the
  activation digest matched;
- published patch-v1.1.0 to candidate patch-v1.2.0 and back to patch-v1.1.0 on
  one data directory. The candidate accepted schema v3 with an activation ack;
  after restoring schema v1 the old image restarted, and the server identity
  hash matched in all three phases. Old-image rollback therefore also restores
  an old-version-readable configuration snapshot;
- final local amd64 static builds of all four binaries, four DEBs, installation
  and runtime in the pinned Debian image, and release-Dockerfile four-command
  plus config-generation smoke. The newest local pre-release image ID is
  `sha256:0995b73a19a64fbdb6204082b78907f50e1210d4318b664e3080dc31eab0c155`;
  it is not the future GHCR digest, which clean release CI will create;
- documentation, contract, Compose, workflow, format, twice-applied-overlay
  idempotency, lockfile, and metadata checks. Checksum-pinned actionlint
  1.7.12, Gitleaks 8.25.1, and Syft 1.50.0 produced zero history/candidate
  secret findings and a source SPDX SBOM; and
- the fixed `cargo-audit 0.22.2` and fixed RustSec database audited 401
  dependencies with zero vulnerability and zero unsound result. The one
  upstream-core `sodiumoxide 0.2.7` unmaintained warning remains disclosed.
  The prior sealed Codex Security review found no high/critical issue, and the
  principal Starry-owned findings now have fixes and regression coverage.

## Publication workflow and post-release gate

- The English/Chinese documentation and feature wording were confirmed on
  2026-08-20. This reviewed release-preparation commit sets
  `RELEASE_STATUS: APPROVED`.
- The exact commit must pass clean GitHub Actions source, security, test,
  Linux, DEB, image, package, and release-candidate jobs. Only then does the
  publish job create the Release and GHCR image and sign the final linux/amd64
  SBOM, provenance, Sigstore bundles, and `SHA256SUMS`.
- after the immutable Starry tag exists, Kessoku must replace its local
  candidate contract marker with that tag/digest (`status: PINNED`) and rerun
  its clean cross-project tests before Kessoku is released.

The local seven-container failure domains, rotation, and restore drills do not
substitute for the user's target-production network, storage, backup media,
scheduler, and certificate-system acceptance. Repeat the same runbook before
expanding production traffic; that is a deployment go-live gate and is not
misrepresented as local patch-candidate evidence. ARM and Windows
compatibility remain explicitly non-blocking for patch-v1.2.0.
