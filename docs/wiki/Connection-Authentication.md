# Connection authentication

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication)

Schema v3 can require a short-lived user JWT before HBBS initiates a new
controller-side connection. This gate is separate from an account API login:
the client places the connection token in the existing RustDesk request field,
and HBBS verifies it again at the signalling boundary.

Do not enable `enforce` merely because a syntactically valid config loads.
Complete issuer integration and a measured `audit` rollout first.

## Covered requests and transports

The same authorization function covers:

| Request | Native TCP 21116 | Secure TCP 21116 | WSS `/ws/id` | UDP 21116 |
| --- | --- | --- | --- | --- |
| Controller `PunchHoleRequest` | verified | verified | verified | unsupported; no response or allocation |
| Direct `RequestRelay` | verified | verified | verified | not dispatched |

Verification happens after frame/protobuf size validation and before target
lookup, punch-request state, Relay choice, UUID creation, or peer delivery.
Denied responses therefore do not reveal whether the requested target exists.
Controlled-endpoint registration does not require a user token.

For official rustdesk-server 1.1.16, native controlled-endpoint `RegisterPk`
and heartbeat traffic still use UDP; its TCP listener returns the upstream
`NOT_SUPPORT` response for that registration message. This is not an
authentication bypass: controller initiation still sends and verifies
`PunchHoleRequest`/`RequestRelay` over native TCP or Secure TCP. A controlled
endpoint on a UDP-free network must use WSS registration instead of expecting
native TCP-only registration.

No protobuf change is required. The exact client-compatible denial contract is
locked by [`contracts/auth/v1/client-compatibility.md`](../../contracts/auth/v1/client-compatibility.md).

## JWT profile

HBBS accepts only:

- compact JWT no larger than `max_token_bytes`;
- `alg=EdDSA`, exact `typ=at+jwt`, and a non-empty explicit `kid`;
- a unique public JWK with `kty=OKP`, `crv=Ed25519`, `use=sig`, and
  `alg=EdDSA`; and
- matching `iss`, `aud`, `token_use`, complete `required_scope`, `sub` and
  `user_id`, plus valid `iat`, `nbf`, and `exp` within configured clock skew.

Symmetric JWKs, private `d` material, duplicate key IDs, algorithm fallback,
trying every key, subject mismatch, and malformed time windows are rejected.

## Start in audit

Copy [`config/config.auth-audit.yaml`](../../config/config.auth-audit.yaml),
replace every example hostname/path, mount the required files read-only, and
keep `mode: audit`:

```yaml
version: 3
connection_auth:
  mode: audit
  issuer: https://kessoku.example
  audience: rustdesk-connect
  jwks:
    file: /run/secrets/starry-auth/jwks.json
    url: https://kessoku.example/api/internal/v1/auth/jwks
    refresh_interval_seconds: 300
    max_stale_seconds: 3600
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
  introspection:
    required: true
    url: https://kessoku.example/api/internal/v1/auth/introspect
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
```

If `jwks.url` is configured, all four JWKS mTLS identity fields are mandatory.
The client trusts only `jwks.ca_file`, presents its configured certificate, and
requires TLS 1.3 and the URL host to equal `jwks.server_name`. Refresh replaces the whole
keyset only after the new document passes validation and is durably cached.
The last-known-good keyset remains active after an invalid refresh, but
verification fails closed after `max_stale_seconds`. A successful remote
refresh writes `<jwks.file>.metadata.json` with the fetched-at time and SHA-256
of the exact keyset. Preserve or restore that mode-restricted sidecar together
with the JWKS; a remote-backed cache without matching metadata has no provable
freshness and enforce mode rejects it after restart.

If `introspection.url` is configured, its four mTLS identity fields are
required even when `required: false`; this client also trusts only its
configured CA, requires TLS 1.3, and requires an exact server-name match. Local signature/claim
failure skips the network call. The request uses Kessoku's strict token-only
JSON DTO; an active response must include the exact locally verified `sub`.
A locally valid token is cached only by its SHA-256 hash and the cache is
bounded. Any configured introspection timeout, TLS error, 5xx, malformed
response, missing/mismatched subject, or inactive result denies/would-deny; it
never fails open.

## Audit gate and alerts

Read `connection_auth` from Agent `GET /control/v1/status`. Record at least:

- `configured_mode`, `effective_mode`, `verifier_state`, `key_count`, and
  `key_age_seconds`;
- metric deltas for `attempts`, `allowed`, `denied`, `audit_would_deny`,
  `cache_hits`, `introspection_requests`, and `introspection_failures`; and
- client version, transport, request kind, and internal reason distribution in
  server logs without retaining a raw token, full JTI, or user secret.

Block enforce rollout when the verifier is not `ready`, a key could become
stale before the next successful refresh, introspection failures are non-zero
without explanation, or any legitimate client creates an unexpected
would-deny. Alert immediately on `denied` or `introspection_failures` growth in
enforce and on any last-known-good config reload rejection.

Remain in audit for at least one complete business cycle and explicitly test:

- missing, malformed, oversize, expired, future, wrong issuer/audience/scope,
  bad signature, unknown key, and stale keyset;
- active, logout/revoked, disabled, deleted, and password-reset users;
- current/previous key overlap, rotation, introspection outage, and recovery;
- native P2P, native Relay, WSS/WSS, and both mixed directions with supported
  real clients; and
- direct `RequestRelay`, nonexistent targets, and UDP no-allocation behavior.

## Enforce and emergency rollback

Canary `mode: enforce` only after the audit gate passes. `--must-login` (or
`MUST_LOGIN=Y`) makes enforce a deployment floor; startup/reload rejects an
incomplete verifier rather than silently allowing connections.

The normal emergency rollback is a local, controlled config change from
`enforce` to `audit` followed by a synchronous activation acknowledgement.
The remote Control API intentionally offers no special bypass or one-click
authentication disable. Keep current and previous public keys available until
the longest issued token lifetime and cache window have elapsed.
