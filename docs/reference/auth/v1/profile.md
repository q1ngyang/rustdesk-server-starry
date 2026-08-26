# Starry connection authentication profile v1

**English** | [简体中文](profile.zh-CN.md)

This profile applies to controller-initiated HBBS connection attempts. It does
not require the controlled endpoint to log in and it does not change the
RustDesk protobuf schema.

## Fixed verification rules

- The only accepted JWT algorithm is `EdDSA`; the protected header must carry
  the exact access-token type `typ: at+jwt` and an explicit `kid` resolving to
  an `OKP`/`Ed25519` public JWK. Symmetric keys, private JWK material,
  algorithm fallback, and trying every key are forbidden.
- Required claims are `iss`, `aud`, `token_use`, `scope`, `sub`, `user_id`,
  `auth_version`, `iat`, `nbf`, `exp`, and `jti`. `user_id` is a non-zero JSON
  integer and `sub` must be its canonical decimal string. Kessoku emits `aud`
  and `scope` as arrays; the configured audience and scope must be present as
  complete values.
- Signature, issuer, audience, token use, time window, maximum token length,
  subject binding, and scope are checked locally before introspection.
- Introspection is keyed in memory by SHA-256 of the token. Raw tokens and full
  JTIs must never enter logs, metrics labels, cache keys, or audit records. The
  request body is exactly `{"token":"..."}`. An active response must include
  a `sub` exactly matching the locally verified subject; omission or mismatch
  fails closed.
- `mode: enforce` fails closed when no valid initial key exists, the keyset is
  stale beyond its configured limit, or required introspection is unavailable.
  A deployment `--must-login`/`MUST_LOGIN=Y` setting is an enforce floor that
  configuration and remote control cannot lower.
- Authentication is invoked by one shared decision point for
  `PunchHoleRequest` and direct `RequestRelay`, after frame/protobuf bounds and
  before target lookup, punch-request recording, Relay selection, or delivery.
- Native TCP, Secure TCP, and `/ws/id` use the same verifier. UDP
  `PunchHoleRequest` remains unsupported and cannot reach authentication,
  target lookup, or allocation.

## Client-compatible denial

No new protobuf enum is introduced. A denied `PunchHoleRequest` uses the
existing `PunchHoleResponse` with `failure=OFFLINE` and the stable,
non-sensitive `other_failure` text `connection authorization failed`. A denied
direct `RequestRelay` uses `RelayResponse.refuse_reason` with the same text.
Internal reason codes are kept in server audit/metrics only; responses do not
reveal whether the target exists, whether a user is disabled, or whether a
token was revoked.

## Deterministic fixture clock

Fixture evaluation time is `2030-01-01T00:00:00Z` (`1893456000`). The active
token is valid around that instant; the expired token is already outside its
window; the wrong-audience token has a valid signature but must fail audience
validation. Fixtures reproduce Kessoku's wire shape: issuer
`https://api.example.test`, audiences `kessoku-api` and `rustdesk-connect`,
token use `access`, scope `connect:initiate`, numeric user ID `42`, and key ID
`kessoku-fixture-2030-01`. The fixture key is test-only public material and
must never be trusted in a deployment.

Test fixtures remain in [`contracts/auth/v1/fixtures`](../../../../contracts/auth/v1/fixtures).
See also the [client compatibility reference](client-compatibility.md).
