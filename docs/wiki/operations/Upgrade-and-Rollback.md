# Upgrade and rollback

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)

An upgrade changes two versions: the official RustDesk Server base and the
Starry patch. A tag such as `1.1.16-patch-v1.3.1` means official server
`1.1.16` plus Starry patch `1.3.1`. Pin that complete tag, and record the image
digest used in production.

## Upgrade rules

- Read both the Starry release notes and the upstream RustDesk Server changes.
- Back up the persistent data directory before pulling or replacing anything.
- Preserve `id_ed25519`; changing it changes the server identity.
- Keep the previous image tag, binary/package, config, and verified Compose
  file available until acceptance is complete.
- Change the binary/image before enabling a new schema or transport feature.
- Roll out one centre at a time; do not run duplicate HBBS instances against
  the same public ports and data directory.
- Mark untested paths as untested. Static checks are not runtime acceptance.

## Read the current patch notes

- [patch-v1.3.1 release-candidate notes](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/RELEASE-NOTES-patch-v1.3.1.md)
- [Changelog](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/CHANGELOG.md)

Patch v1.3.1 adds role-authorized FastMedia Relay UDP, telemetry schema 2,
schema v5, and SP1 pairing on top of the frozen v1.3.0 candidate Relay probes,
two-endpoint RTT/jitter/loss scoring, trusted HBBR load telemetry, and signed FastCompat authorization for
compatible Akari/Kessoku deployments. Official clients and schema v1-v4 keep
their prior registration and reliable Relay flow. FastMedia and pairing are
independent and default-off; this candidate remains blocked until the real
fallback/re-entry release gate passes.

## patch-v1.3.1 upgrade and downgrade

Before changing containers, bind the complete Control state below one absolute
`STARRY_PERSIST_ROOT` and each Relay below its absolute `RELAY_DATA_DIR`.
Back up `hbbs`, `config`, `control/state`, `control/identity`,
`control/generated`, `control/shared`, `relay-secrets`, and every Relay
`starry/enrollment` directory as one permission-preserving set. Verify the
backup on an isolated host; never start the original and a cloned Relay
identity together. `pull`, `force-recreate`, and `down`/`up` preserve identity
only with the same explicit mounts. Treat `down -v`, a different relative path,
overlay/tmpfs state, or a host-identity mismatch as a failed preflight.

Upgrade in this order:

1. Roll HBBR v1.3.1 first with its reliable TCP/WS path unchanged and FastMedia
   policy disabled. Verify authenticated telemetry schema 2, UDP listener
   health, ordinary Native/WSS/mixed Relay, and official clients.
2. Roll HBBS with the existing schema v4 or a schema-v5 document whose two
   Fast switches are false. Verify the exact Relay Quality v1 digest and
   ordinary GEO/failover decisions.
3. Roll Control Agent, verify the schema/OpenAPI fixtures, SP1 capability, and
   read-only inventory. Pair/adopt existing identity only through an explicit
   reviewed command; never allow `pair` to overwrite it. For certificates
   addressed by DNS, pass the exact Kessoku-allowlisted name with
   `--tls-server-name`; a rotate must preserve the installed Agent listen,
   local-control address, size limit, and write policy.
4. Canary FastCompat, then FastMedia on a bounded Relay/Akari cohort. Every
   failed bind, UDP block, listener restart, or rate limit must leave the
   reliable desktop session alive.

Schema v5 adds only:

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

Each eligible Relay also declares `fast_media_udp_port` beside its authenticated
`/ws/telemetry` endpoint. Do not enable FastMedia until fresh telemetry reports
protocol 1 and healthy on the same port.

For v1.3.1→v1.3.0, first set `fast_media_v1_enabled: false` and obtain a
synchronous activation ACK. Then preview and export:

```console
starry-control-agent config downgrade --to-schema 4 --preview
starry-control-agent config downgrade --to-schema 4 \
  --output /safe/config-v4.yaml
```

The command fails while any FastMedia authorization, allocation, active stream,
or unexpired last grant remains, and while any Agent/Relay certificate has less
than ninety days remaining. Rotate certificates before entering the rollback
window. Export is no-clobber and removes only v5 fields. Review its digest,
activate the schema-v4 output, roll HBBS, then HBBR using the generated
non-secret `relay-compat.env`, then Control Agent. patch-v1.3.0 continues to
read Agent v1 YAML/PEM/JWKS and ordinary telemetry secret files; it ignores but
must not delete pairing/enrollment state. Upgrading back to v1.3.1 reuses the
same identity and requires fresh UDP health before reenabling FastMedia.
The compatibility parser preserves any trailing Base64 padding in public
`KEY` values.

Treat the Kessoku downgrade as a coordinated compatibility change. Kessoku
v3.0.7 freezes config schema ≤4 and telemetry schema 1, so it must not be
pointed directly at a schema-v5/telemetry-v2 Starry v1.3.1 inventory. Activate
the exported schema-v4 configuration and roll Starry to v1.3.0 before starting
Kessoku v3.0.7. Reverse the sequence on upgrade: restore Kessoku v3.0.8 and
Starry v1.3.1, verify fresh telemetry-v2 inventory, and only then re-enable
FastMedia.

The remaining numbered workflow below documents the original v1.3.0
commissioning sequence and still applies to Relay Quality/Profile activation.

## 1. Inventory and backup

From the deployment directory:

```sh
set -eu
date -u
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml config --images
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}} {{json .Image}}'

backup_dir="../rustdesk-starry-backup-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$backup_dir"
cp -a data "$backup_dir/data"
cp -a .env compose.yaml "$backup_dir/"
sha256sum "$backup_dir/data/id_ed25519" \
  "$backup_dir/data/id_ed25519.pub" \
  "$backup_dir/data/starry/config.yaml"
```

Protect the backup: it contains the private server identity and may contain
database or account-related state. Verify that files are non-empty and that
the backup resides outside the live bind directory.

For an optional third-party API, follow that project's database-consistent
backup procedure separately. Starry cannot guarantee another project's state
format or migration behaviour.

## 2. Prepare a candidate configuration

For patch v1.2.0 to v1.3.0, keep the existing schema v3 (or earlier) file for
the first binary/image replacement. Prepare schema v4 as a separate candidate:

```yaml
version: 4

# Existing relay_servers, secure_tcp, mmdb, and geo sections stay here.

# Preserve existing connection_auth and websocket_signal settings verbatim.

relay_quality:
  enabled: false
  # Enable only after the legacy and official-client baseline passes.

fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
  # Enable only after quality, auth, and secure signalling canaries pass.
```

Do not overwrite the active config yet or weaken an existing authentication
mode merely to satisfy a rollout date.

## 3. Pull and inspect without starting

Set the new immutable version in `.env`:

```dotenv
STARRY_VERSION=1.1.16-patch-v1.3.0
```

The supplied Compose files use that same Starry image version for HBBS and the
bundled HBBR. HBBR preserves the upstream byte-forwarding path and adds a bounded
public probe plus authenticated telemetry channel; there is no separately updated HBBR image tag.

Then:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml images
```

If your policy pins digests, compare the pulled digest with the reviewed
release/package value. Do not infer identity from a mutable `latest` tag.

## 4. Replace only the intended services

```sh
docker compose --env-file .env -f compose.yaml up -d hbbs hbbr
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr
```

With the old schema v3 (or earlier) config, verify:

- both services remain stable and use the existing key;
- native registration works;
- API-authenticated native Secure TCP works when an API is deployed;
- P2P and native Relay work; and
- the expected native Geo and failover decisions remain unchanged.

Stop here if the native baseline regresses.

## 5. Introduce schema v4 with Relay quality off

Install the candidate as `data/starry/config.yaml`, then invoke the authenticated
Control Agent `POST /control/v1/runtime:reload` operation and inspect logs:

```sh
docker logs --tail 200 rustdesk-starry-hbbs
```

The response and logs must report a new generation, matching source/effective
digests, and successful subsystem acknowledgements. Validate native and every
previously enabled WSS/mixed path again. Process survival alone is not
acceptance; an invalid candidate retains the prior last-known-good generation.

## 6. Canary Relay quality separately

Upgrade HBBR before HBBS quality activation. Through inventory, require
`relay_probe_protocol: 1`, `relay_load_protocol: 1`, fresh telemetry, and one
unique health endpoint for every quality Relay. A legacy HBBR must be named in
`legacy_fallback_relays`; a version string alone is not capability evidence.
Keep at least two compatible Relays online, then enable `relay_quality` on one
centre or client cohort. Verify that:

1. official clients still receive and use the legacy `relay_server` value;
2. compatible Akari peers receive only the GEO primary in stage 1; a good
   primary creates no other HBBR probe, while either endpoint reporting poor
   quality causes both endpoints to receive the remaining candidates together;
3. HBBS returns the same selected Relay in the legacy field and the optional
   decision extension;
4. Kessoku/Control Relay state exposes capability versions, telemetry
   observation age/stale state, and accepted/late/invalid/binding-mismatch
   counters without client identifiers;
5. P2P success cancels the allocation, force-auto/WSS/symmetric-NAT paths wait
   for a final decision, and an official peer produces an explicit partial
   single-endpoint result;
6. loss, overload, stale health, stage reordering, duplicate/late reports, and
   rate limits select or fall back deterministically; and
7. reconnects within one network-prefix pair do not flap below the configured
   hysteresis margin.

Clients never query the Control Agent. Keep `relay_quality.enabled: false` if
the deployed Akari protocol version or canonical protobuf SHA-256 does not
match the FROZEN release contract. Do not deploy Akari wire support from an
unfrozen branch.

## 7. Commission the Agent and authentication separately

1. Deploy the Linux Control Agent with `write_enabled: false` and a private
   listener. Verify mTLS CA/URI-SAN and service-JWT audience/azp/scope denies.
2. Verify read-only status/Relay/config endpoints and repeated side-effect-free
   allocation simulation. Do not enable writes yet.
3. In staging only, enable writes and exercise apply, rollback, HBBS outage,
   Agent restart, disk drift, and recovery blocking.
4. Deploy the compatible client token issuer, public Ed25519 JWKS, and mTLS
   introspection endpoint. Keep HBBS `connection_auth.mode: audit`.
5. Run audit for a full business cycle and complete native TCP, Secure TCP,
   WSS, direct Relay, logout/revoke/disable/password-reset, key rotation, and
   dependency-failure tests.
6. Canary `enforce` on one instance or user cohort. Expand only with measured
   evidence; UDP initiation stays unsupported and must never allocate.

## 8. Canary Profile activation before FastCompat

Upgrade every target HBBS and its Control Agent first. Confirm
`profile_activation_lease: 1`, then upgrade Kessoku to keep route leases by
HBBS `instance.id`, call `/peers:verify` on the issuing instance, and redact
activation IDs/public keys/leases. Do not enable Akari switching yet.

Canary Akari only after its client-side state machine keeps the old Profile
committed until a successful Ready ACK exactly matches activation ID/epoch and
contains a 32-byte lease plus non-zero generation. Exercise Native UDP/TCP,
WSS old-reader exit, reordered messages, A→B→A, legacy official clients, and at
least two HBBS nodes. Expand only while stale/rate/capacity/TTL counters and
HBBR session/disconnect pressure remain within baseline.

The complete protocol and rollout contract is
[Profile Activation Lease v1](../../reference/PROFILE-ACTIVATION-LEASE-v1.md).

## 9. Canary FastCompat authorization last

Leave `allow_fast_media_v1` out of configuration; patch-v1.3.0 hard-codes it
false and does not ship a FastMedia Relay transport. In a separate planned
change, set `fast_mode.relay.fast_compat_enabled: true` only after Relay
quality, exact auth allow, and Secure TCP/WSS have passed canary acceptance.

Verify that:

1. `GET /control/v1/capabilities` reports
   `fast_relay_authorization: 1`, and `/relays.fast_relay` counters are present;
2. official clients and Akari sessions without a quality offer receive no
   grant and still complete the standard Relay flow;
3. Akari verifies the grant with the existing HBBS public key, matches the
   session UUID/expiry/bitrate, and observes `allow_fast_media_v1: false`;
4. the target `RequestRelay` and controller `RelayResponse` contain identical
   signed bytes after the same final Relay-quality decision;
5. a same-session retry reuses the exact bytes without extending expiry, while
   a changed source/target binding receives no grant; and
6. Kessoku audit and service logs contain policy/counters only—never signing
   keys, connection tokens, session UUIDs, or signed grants.

Use the complete checklist in
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).

## Feature rollback

If Profile activation regresses, stop new Akari switches first and retain each
client's last committed Profile. Send best-effort `DeactivatePeer` to every
issuing HBBS with that node's lease/generation, then wait at least 45 seconds
plus WSS idle/drain. The server state is in memory only; do not delete peer
records or rotate public keys. An older server returns no matching enhanced
ACK, so a conforming client fails closed to its previous Profile.

If FastCompat regresses, first set
`fast_mode.relay.fast_compat_enabled: false` and require a synchronous reload
acknowledgement. New sessions immediately receive no grant and continue the
standard reliable Relay path. Wait longer than the configured authorization
TTL before considering all previously issued grants expired; no HBBR stream is
forcibly migrated.

If only staged coordination regresses, first set `relay_quality.strategy:
eager` and require a synchronous reload; this keeps frozen v1 wire bindings
while restoring eager-all-candidates behaviour for new allocations. If Relay
quality itself regresses, set `relay_quality.enabled: false`, perform a
synchronous runtime reload, and confirm that new allocations use only the
legacy Geo/failover Relay choice. Existing relay byte streams are not migrated;
let them drain or reconnect according to the incident policy. Wait at least
one `report_timeout_ms` before rolling back an HBBR image. If only one HBBR
must be reverted, first move it into `legacy_fallback_relays`, reload, confirm
it is no longer a quality candidate, wait for in-flight offers, then replace
it. Roll back HBBS after HBBR capability withdrawal, not before.

If only WebSocket Signal regresses and native behaviour remains sound:

1. set `websocket_signal.enabled: false`;
2. run the authenticated `POST /control/v1/runtime:reload` operation;
3. confirm the management response and HBBS drain log;
4. disable WebSocket on clients or remove the rollout policy; and
5. re-verify native registration, P2P, Secure TCP, and native Relay.

Keep the public Nginx locations closed or unused according to your rollback
policy. Existing WSS sessions are drained when the feature is disabled.

For connection-authentication regression, make the local controlled change
from `enforce` to `audit` and require a synchronous reload acknowledgement.
There is intentionally no remote one-click authentication bypass. Stop or
return the Agent to read-only independently; HBBS/HBBR continue using the last
active configuration. An operation in `manual_intervention_required` blocks
new writes until disk bytes and runtime digests are reconciled.

## Image rollback

Restore the previous `.env` and configuration from the reviewed backup:

```sh
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/.env .env
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/compose.yaml compose.yaml
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/data/starry/config.yaml \
  data/starry/config.yaml

docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d hbbs hbbr
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr
```

Replace the placeholder backup path only after verifying its resolved location
and contents. Do not copy an entire old data directory over a live deployment
unless data-format compatibility requires it and the services are stopped.
Usually preserving the current key/state and restoring only the compatible
config plus previous image is the safer first rollback.

After rollback, repeat the native and applicable API/Relay acceptance tests.
Before starting patch-v1.2.0, restore schema v3 or earlier without
`fast_mode` or `relay_quality`; patch-v1.2.0 rejects schema v4. Before
patch-v1.1.0, restore schema v2 or v1. Preserve Agent audit/transaction state
separately until the incident is closed.

## DEB or standalone binary upgrade

For DEB packages, download the matching architecture, verify the release
checksum, back up `/var/lib/rustdesk-server-starry`, then install the packages
with your package manager. Restart and inspect one service at a time:

```sh
sudo systemctl restart rustdesk-server-starry-hbbs
sudo systemctl status rustdesk-server-starry-hbbs --no-pager
sudo journalctl -u rustdesk-server-starry-hbbs -n 200 --no-pager

sudo systemctl restart rustdesk-server-starry-hbbr
sudo systemctl status rustdesk-server-starry-hbbr --no-pager
```

The HBBR package is built from the pinned official revision, preserves the
upstream byte-forwarding path, and adds a bounded public probe plus authenticated telemetry channel.
Rollback requires the previous package files or repository snapshot; do not
assume a package cache still contains them.

For standalone binaries, keep versioned filenames and atomically update a
service symlink or service path. Never overwrite the only known-good binary
before its checksum and backup are recorded.

## Stop and roll back when

- the Starry config is rejected or unexpectedly falls back;
- existing keys change or persistent files disappear;
- native registration or native Relay regresses;
- Secure TCP fails for previously working authenticated clients;
- no eligible Relay exists for a required transport;
- quality offers or decisions disagree with the legacy Relay field;
- WSS health never becomes ready after the documented threshold; or
- a connection-auth request bypasses the shared gate or an expected client is
  unexpectedly denied;
- JWKS/introspection failure appears to fail open;
- Agent apply lacks a matching runtime acknowledgement or enters
  `manual_intervention_required`; or
- a two-client session cannot complete the required control/data test.

Publication checks and CI results are useful release evidence, but they do not
replace acceptance on your DNS, certificates, proxies, networks, and clients.
