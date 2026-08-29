# Control Agent

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)

`starry-control-agent` is an optional Linux management component for one local
Starry HBBS instance. Kessoku or another controller talks to the Agent over
HTTPS with mutual TLS and a scoped service JWT. Only the Agent talks to HBBS,
using the bounded loopback `STARRYCTL/1` protocol on `127.0.0.1:21115`.

The Agent is not an account API and is not required for the HBBS data plane.
Stopping it removes management access without disabling the last active HBBS
configuration.

## Security boundary

Every remote request requires both:

1. a client certificate chaining to `tls.ca_file` whose URI SAN exactly
   matches one configured `allowed_client_uri_sans` item; and
2. an EdDSA service JWT from the separate `service_jwt.jwks_file`, with maximum
   five-minute lifetime, expected issuer, `azp`, requested scope, and audience
   `urn:starry-control:<instance_id>`.

Connection JWT keys and service JWT keys are deliberately separate. The API
surface is fixed by [`contracts/control/v1/openapi.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/contracts/control/v1/openapi.yaml).
There is no arbitrary command, arbitrary path, Docker/systemd control, URL
fetch, shell, or raw `21115` proxy.

Keep the Agent listener on host loopback for a same-host controller or on a
firewall-restricted private management address. Never publish `21120` through
the public RustDesk or reverse-proxy listener.

## Linux installation

The Linux archive/container and `rustdesk-server-starry-control-agent` DEB
contain the Agent. The DEB installs but does not automatically enable its
systemd service. No Windows Agent artifact is published in v1.2.0 because the
atomic transaction implementation is release-supported only on Unix filesystems.

Start from [`config/control-agent.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/control-agent.example.yaml):

```yaml
version: 1
instance_id_file: /var/lib/rustdesk-server-starry/control-agent-instance-id
listen: 127.0.0.1:21120
local_control:
  address: 127.0.0.1:21115
  token_file: /etc/rustdesk-server-starry/local-control.token
config:
  write_enabled: false
  path: /etc/rustdesk-server-starry/managed/config.yaml
  backup_dir: /var/lib/rustdesk-server-starry/config-history
  max_bytes: 1048576
```

Install the server certificate/key, client CA, and service public JWKS at the
configured paths. The Agent service user needs read access to those files,
read/write access only to the managed config and state directory, and no
access to Docker socket or host service-control interfaces.

HBBS and the Agent must share one independent local-control token. The DEB
creates `/etc/rustdesk-server-starry/local-control.token` with mode `0600` and
configures HBBS through `STARRY_LOCAL_CONTROL_TOKEN_FILE`. For containers,
create `secrets/local-control.token` as 32–256 base64url characters, make it
readable only by the Agent numeric UID, and mount the same read-only file into
both services. An absent, broadly readable, malformed, or mismatched token
fails closed. The token is never accepted by the remote Control API.

For write-enabled atomic replacement, the existing managed config must be
owned by the Agent service UID and primary GID. Making a root-owned file merely
group-writable is insufficient: atomic rename creates a new inode, and the
least-privilege Agent deliberately refuses to start with writes enabled if it
cannot preserve the exact owner. The DEB sets the managed file to `rustdesk-starry:rustdesk-starry`
mode `0640`; a container bind mount must use the numeric UID/GID from its
Compose environment. The config must not be group/other-writable, and every
parent component must be a real, confined directory; each transaction binds
replacement to the parent device/inode observed with the source bytes.
The state root and its generated subdirectories must be Agent-owned mode
`0700`; durable JSON/YAML files are single-link regular files at mode `0600`.
TLS keys, the Agent YAML, and service JWKS remain
root-owned and read-only to the Agent.

For containers, use [`examples/control-agent/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/control-agent/compose.yaml).
The sidecar shares the HBBS network namespace solely so `127.0.0.1:21115`
remains local. It shares the Starry config volume read/write while HBBS mounts
the same volume read-only. The example Agent binds host loopback and starts in
read-only mode.

## Read-only commissioning

Leave `write_enabled: false` for initial deployment. The Agent authenticates
requests normally, advertises only read/simulation capabilities, and returns
404 for plan, apply, rollback, and runtime reload.

Verify in order:

1. no client certificate, wrong CA, and wrong URI SAN fail;
2. missing/expired/wrong audience, `azp`, or scope service JWT fails;
3. `GET /control/v1/capabilities`, `/status`, `/relays`, `/config/schema`, and
   `/config` return structured data; `/config` includes the exact managed UTF-8
   YAML and strong ETag but never dereferences secret-file references; each
   Relay entry reports the exact HBBR Starry version last observed through its
   WSS handshake, or `null` for a legacy or unprobed endpoint;
4. a service-only JWT with `starry.peer.verify` can call
   `POST /control/v1/peers:verify`; it returns only the instance ID and whether
   the exact RustDesk ID/machine-UUID pair exists, never peer metadata, keys,
   addresses, or a registry listing;
5. `POST /allocations:simulate` returns a trace while repeated calls leave
   rotation/health/generation and production counters unchanged; and
6. the listener is unreachable from public networks and HBBS `21115` remains
   loopback-only.

Each response includes `X-Request-ID`; a valid W3C `traceparent` is retained in
durable audit records for mutations. Do not send a raw user connection token
to this API.

## Enabling configuration writes

Only set `write_enabled: true` after staging apply/rollback and outage recovery
have passed on the target filesystem. A normal change uses:

1. `GET /config` and retain the strong ETag over exact disk bytes;
2. `POST /config:validate` with the YAML candidate;
3. `POST /config:plan` with `If-Match`; review risk, changes, digest, instance,
   generation, and expiry;
4. `POST /config:apply` with the same `If-Match`, candidate digest, plan ID,
   and a unique 16–128 byte `Idempotency-Key`; and
5. poll `GET /operations/{id}` until `succeeded`, then compare its activation
   acknowledgement to `GET /config` and `/status`.

The Agent rejects concurrent/stale plans and external disk drift. An
idempotency key can replay only the exact same mutation. A successful apply is
reported only after atomic disk replacement and HBBS acknowledgment of source
digest, effective digest, generation, and every required subsystem.
Terminal operation, idempotency, audit, and recovery records expire after 24
hours and are additionally constrained by count and a 256 MiB aggregate state
budget. Pending/running/manual-intervention records are never pruned
automatically; a full protected store fails closed until an operator
reconciles it.

Rollback is a new audited transaction selected from `/config/history`; it
does not erase history. `restart_required` plans are not applied and the Agent
never invokes Docker or systemd.

## Recovery runbook

Alert on any operation in `rolled_back`, `failed`, or
`manual_intervention_required`, on runtime/disk drift, and on Agent audit/state
persistence errors.

For an ordinary automatic rollback, verify that disk ETag and HBBS source
digest again match the pre-operation values before retrying with a new plan and
idempotency key. For `manual_intervention_required`:

1. disable/stop the Agent or return its config to read-only;
2. preserve `config-history/operations`, `audit`, `recovery`, `revisions`, and
   `idempotency` as incident evidence;
3. compare the managed file's exact bytes/owner/mode and the local HBBS runtime
   generation/digests with the operation recovery manifest;
4. restore the reviewed last-known-good bytes and perform a local acknowledged
   reload; and
5. restart the Agent only after disk/runtime consistency is proven.

Never delete the state directory merely to clear the block; doing so discards
the evidence needed to know which bytes and runtime were active.
