# Starry Pairing v1

**English** | [简体中文](STARRY-PAIRING-v1.zh-CN.md)

Starry Pairing v1 (`SP1`) bootstraps either a Control Agent or one Relay with a
short-lived, single-use code. It is not a long-lived control protocol. After
claim, the Control Agent continues to use its existing TLS 1.3 mTLS and scoped
service-JWT boundary; each Relay continues with its node identity, independent
telemetry secret, signed Fast Relay grants, and normal HBBR data plane. Broker,
Kessoku, or Control Agent availability is not required for an established
RustDesk session.

The canonical documents are defined by
[`contracts/starry-pairing/v1/pairing.schema.json`](../../contracts/starry-pairing/v1/pairing.schema.json).

## SP1 code and claim binding

The code has the form `SP1.<base64url-no-pad canonical JSON>` and carries:

- protocol version 1 and purpose `control-agent` or `relay`;
- exact HTTPS Broker origin and its SHA-256 SPKI pin;
- enrollment UUID, approved configuration digest, expiry, and a 256-bit random
  secret.

The client accepts a code only from an interactive stdin prompt or a regular
mode-0600 file. It rejects command-line/environment code values, plaintext
HTTP, origin drift, pin mismatch, unknown fields, wrong purpose, expiry, and
oversized input. Claims bind the enrollment, purpose, configuration digest,
secret, request digest, and locally generated key/CSR fingerprint.

The first valid claim atomically binds that key. A lost response may be
retrieved only with the exact same request/key during the Broker recovery
window. Another key, changed CSR, purpose swap, digest drift, replay after
revoke/expiry, or endpoint drift fails closed. Raw codes, private keys,
telemetry secrets, CSR contents, and complete certificate chains are not
logged or exported as metrics.

The client verifies the pinned Broker TLS connection, returned enrollment and
request bindings, key/certificate match, CA signature, and certificate
validity before writing any identity.

## Control Agent pairing

The Control Agent generates its server private key and CSR locally. The Broker
returns a server certificate, client CA, allowed client URI SANs, service JWKS,
JWT issuer/audience, exact Agent origin, instance UUID, and centre public key.
The generated runtime YAML uses only Control Agent v1 fields already accepted
by patch-v1.3.0.

```console
starry-control-agent pair
Paste pairing code:
```

`pair` requires empty destinations and never overwrites an existing identity,
instance UUID, local token, or YAML. `adopt` explicitly imports management for
an existing Starry instance while retaining the instance identity.
`rotate` changes the managed certificate material only through a new bound SP1
claim. All modes support explicit `--state-dir`, `--identity-dir`, `--output`,
`--shared-dir`, `--managed-config`, `--backup-dir`, `--listen`,
`--local-control-address`, and optional `--broker-ca-file` paths. When the
Agent certificate is used through a DNS name, pass that exact allowlisted name
with `--tls-server-name` (or `STARRY_CONTROL_AGENT_TLS_SERVER_NAME`). It is
encoded as a DNS SAN in the locally generated CSR. IP literals, ports, URL
syntax, empty labels, and invalid DNS labels are rejected locally.

Pairing is crash-idempotent: an exact already-installed file is accepted on
retry, while any different byte content fails rather than being overwritten.
An interrupted first `pair` also resumes the instance UUID already bound in
its durable pending record; it does not silently generate a second identity.
`rotate` preserves the installed Agent listen address, local-control address,
managed-config size limit, and write policy after validating all generated
Agent-v1 path bindings. Broker-returned trust material is still replaced only
by the new bound response. The generated local control token remains between
Agent and HBBS and is never returned to the Broker.

## Relay enrollment

Kessoku may broker an SP1 claim, but the authenticated Starry Control Agent is
the authority for approved Relay endpoint, pool, profile, capacity, draining
state, and configuration digest. Control API exposes bounded
`prepare`, `complete`, health-gated `activate`, `revoke`, list, and get operations under
`/control/v1/relay-enrollments`; write operations require
`starry.relay.enroll`, write-enabled Agent policy, mTLS, service JWT, and normal
idempotency controls. API responses never expose an SP1 secret, private key,
or stored telemetry secret after the one required claim response. An exact
same-key/same-CSR retry may recover a lost claim response for at most ten
minutes; after that window even the same request cannot retrieve the bundle
secret again.

The Relay generates its node key locally and uses the separate utility so the
upstream `hbbr` CLI remains unchanged:

```console
starry-relayctl enroll --data-dir /var/lib/rustdesk-server-starry/relay
Paste pairing code:
```

The installed `starry/enrollment` directory contains node identity and
certificate, Relay CA, centre public key, per-Relay telemetry secret, approved
runtime JSON, a host binding, and `relay-compat.env`. The compatibility file
contains only a public key, non-secret limits/endpoints, and the telemetry
secret file path—never the secret value. It is restricted to a fixed key
allowlist by `starry-relay-entrypoint`.
The non-secret completion marker also retains the enrollment ID and approved
configuration digest. If the process stops after the marker is durable but
before pending-file cleanup or the success response, an exact retry validates
the installed key/certificate and those bindings, removes only matching
pending files, and returns the same completed state. A changed code, key, CSR,
purpose, or digest is never adopted as recovery.

Revocation durably changes the registry state before retiring the current
per-Relay certificate, telemetry secret, and claim marker. Retirement is
bound to the exact claim, leaves the claim marker until last, and is retryable
after partial cleanup. A later enrollment for the same node may replace only
a claim whose matching registry record is `revoked` or `expired`; active,
unknown, mismatched, and concurrently changed claims remain fail-closed. The
compatibility parser also preserves standard Base64 padding in the public
RustDesk `KEY` instead of treating a trailing `=` as a delimiter.

The Relay-only Compose profile defaults `RELAY_REQUIRE_ENROLLMENT=1` (mapped to
`STARRY_REQUIRE_RELAY_ENROLLMENT=1`) before enrollment. Run `starry-relayctl`
as a one-shot command against the same bind mount, then start HBBR. This
persistent operator intent makes an empty, deleted, or wrongly mounted
`RELAY_DATA_DIR` fail before HBBR can silently start as a fresh manual Relay.
Manual/official HBBR remains supported only when the operator explicitly sets
the flag to `0`; do not reuse that opt-out for a previously enrolled Relay.

An enrolled Relay cannot add itself to a GEO pool or alter its approved public
endpoint. `activate_after_health` is immutable high-risk pre-authorization;
otherwise the enrollment remains pending for the existing Agent configuration
transaction. A pre-authorized claim first enters `claimed_pending_health`.
After the normal `validate -> plan -> apply` transaction succeeds, the caller
posts only its operation ID, resulting config generation and health snapshot ID
to `relay-enrollments:activate`. The Agent verifies the durable activation ACK
and independently rereads HBBS inventory. It requires the exact approved Relay,
Native health, version, and—where selected by the profile—fresh authenticated
schema-2 telemetry, WSS health, capacity/draining values, and FastMedia UDP
capability/port/health before atomically entering `active`. Exact retries are
idempotent; stale generations, endpoint drift, partial ACKs and changed evidence
are rejected. DNS, certificates, firewalls, and externally observed Native,
WSS, telemetry, and UDP health remain deployment prerequisites.

## Persistent layouts

Container state must be under the explicitly mounted roots:

```text
STARRY_PERSIST_ROOT/
  control/state/       instance and bounded enrollment registry
  control/identity/    Agent identity, client CA and Relay CA
  control/generated/   Agent v1 runtime YAML
  control/shared/      HBBS/Agent local token
  relay-secrets/<id>/  per-Relay material
  config/              active config, snapshots and history
  hbbs/                RustDesk identity and HBBS state

RELAY_DATA_DIR/
  starry/enrollment/   Relay identity, config and compatibility export
```

Pairing and service startup reject relative paths, symlinks where regular
files/directories are required, unsafe owner/mode, overlay/tmpfs container
layers, missing explicit mounts, and Relay host-identity mismatch. The same
Relay enrollment directory cannot be active on two hosts. `docker pull`,
`force-recreate`, and `down`/`up` preserve identity only when the same explicit
host roots are mounted. `down -v`, a changed relative host path, or copying one
identity to concurrent hosts is an operator error and must fail preflight.

Native/DEB defaults separate configuration and state:

```text
/etc/rustdesk-server-starry/
/var/lib/rustdesk-server-starry/
```

Package upgrade/downgrade does not overwrite identities. Removal of identity
requires an explicit operator purge outside the normal upgrade path.

## Manual compatibility and downgrade

Existing hand-written Agent YAML/mTLS/JWT and manual HBBR public-key/telemetry
secret-file deployments remain supported; pairing is optional. patch-v1.3.0
can read the generated Agent v1 YAML, PEM, and JWKS. An enrolled HBBR can be
temporarily run as patch-v1.3.0 with its `relay-compat.env`, retaining ordinary
Relay and telemetry while ignoring enrollment and FastMedia state.

Before a v1.3.1→v1.3.0 rollback, certificates must have at least ninety days
remaining. Disable and drain FastMedia, export schema v4, then replace HBBR,
HBBS, and Agent in that order. Old binaries must ignore, not delete, the
preserved SP1/enrollment state. Upgrading back to v1.3.1 reuses the same
identity and requires fresh health before FastMedia can be enabled again.
