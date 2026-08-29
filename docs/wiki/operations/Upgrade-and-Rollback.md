# Upgrade and rollback

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)

An upgrade changes two versions: the official RustDesk Server base and the
Starry patch. A tag such as `1.1.16-patch-v1.2.2` means official server
`1.1.16` plus Starry patch `1.2.2`. Pin that complete tag, and record the image
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

- [patch-v1.2.2 release notes](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/RELEASE-NOTES-patch-v1.2.2.md)
- [patch-v1.2.1 release notes](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/RELEASE-NOTES-patch-v1.2.1.md)
- [patch-v1.2.0 release notes](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/RELEASE-NOTES-patch-v1.2.0.md)
- [Changelog](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/CHANGELOG.md)

Patch v1.2.2 adds private peer-registry verification for Kessoku device
discovery without a configuration migration. Upgrade the center HBBS and its
Control Agent together; relay-only nodes do not provide this endpoint. Patch
v1.2.1 added Relay version reporting without a configuration migration.
Patch v1.2.0 added schema v3 last-known-good activation, strict optional
connection JWT audit/enforcement, immutable Relay snapshots, side-effect-free
simulation, and an optional least-privilege Linux Control Agent. Schema v1/v2
remain accepted with connection authentication off.

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

For patch v1.1.0 to v1.2.0, keep the existing schema v2 (or v1) file for the
first binary/image replacement. Prepare schema v3 as a separate candidate:

```yaml
version: 3

# Existing relay_servers, secure_tcp, mmdb, and geo sections stay here.

connection_auth:
  mode: off
  # Add reviewed issuer/JWKS/introspection values before moving to audit.
```

Keep existing WebSocket settings unchanged. Do not overwrite the active config
yet, and do not add an authentication issuer merely to satisfy a rollout date.

## 3. Pull and inspect without starting

Set the new immutable version in `.env`:

```dotenv
STARRY_VERSION=1.1.16-patch-v1.2.2
```

The supplied Compose files use that same Starry image version for HBBS and
the bundled HBBR with its upstream relay data path and Starry version header.
There is no separately updated HBBR image tag.

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

With the old schema v2 or v1 config, verify:

- both services remain stable and use the existing key;
- native registration works;
- API-authenticated native Secure TCP works when an API is deployed;
- P2P and native Relay work; and
- the expected native Geo and failover decisions remain unchanged.

Stop here if the native baseline regresses.

## 5. Introduce schema v3 with authentication off

Install the candidate as `data/starry/config.yaml`, then invoke the authenticated
Control Agent `POST /control/v1/runtime:reload` operation and inspect logs:

```sh
docker logs --tail 200 rustdesk-starry-hbbs
```

The response and logs must report a new generation, matching source/effective
digests, and successful subsystem acknowledgements. Validate native and every
previously enabled WSS/mixed path again. Process survival alone is not
acceptance; an invalid candidate retains the prior last-known-good generation.

## 6. Commission the Agent and authentication separately

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

Use the complete checklist in
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).

## Feature rollback

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
Before starting patch-v1.1.0, the restored file must be schema v2 or v1;
patch-v1.1.0 does not understand schema v3. Preserve the v1.2 Agent audit and
transaction state separately until the incident is closed.

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

The HBBR package is an unmodified upstream HBBR built from the pinned official
revision. Rollback requires the previous package files or repository snapshot;
do not assume a package cache still contains them.

For standalone binaries, keep versioned filenames and atomically update a
service symlink or service path. Never overwrite the only known-good binary
before its checksum and backup are recorded.

## Stop and roll back when

- the Starry config is rejected or unexpectedly falls back;
- existing keys change or persistent files disappear;
- native registration or native Relay regresses;
- Secure TCP fails for previously working authenticated clients;
- no eligible Relay exists for a required transport;
- WSS health never becomes ready after the documented threshold; or
- a connection-auth request bypasses the shared gate or an expected client is
  unexpectedly denied;
- JWKS/introspection failure appears to fail open;
- Agent apply lacks a matching runtime acknowledgement or enters
  `manual_intervention_required`; or
- a two-client session cannot complete the required control/data test.

Publication checks and CI results are useful release evidence, but they do not
replace acceptance on your DNS, certificates, proxies, networks, and clients.
