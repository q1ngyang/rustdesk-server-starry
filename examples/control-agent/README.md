# Control Agent Compose example

**English** | [简体中文](README.zh-CN.md)

This example is an opt-in Linux sidecar deployment. Read the full
[Control Agent guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent)
before starting it. The checked-in configuration is read-only
(`write_enabled: false`) and binds the mTLS API to host loopback.

Set `STARRY_PERSIST_ROOT` in `.env` to one host directory. The default is
`./persist`; an absolute production example is
`/www/wwwroot/rustdesk/starry`. Compose mounts its children separately, so a
single tidy host root does not merge the container permission domains:

```text
STARRY_PERSIST_ROOT/
├── hbbs/
├── config/
├── auth/
│   ├── secrets/
│   └── cache/
└── control/
    ├── secrets/
    ├── shared/
    └── state/
```

Prepare the tree without placing real secrets in the repository:

```sh
cp .env.example .env
starry_persist_root=/www/wwwroot/rustdesk/starry
starry_control_uid=65532
starry_control_gid=65532

sudo install -d -m 0750 -o root -g "${starry_control_gid}" \
  "${starry_persist_root}"
sudo install -d -m 0700 -o root -g root \
  "${starry_persist_root}/hbbs" \
  "${starry_persist_root}/auth/secrets" \
  "${starry_persist_root}/auth/cache"
sudo install -d -m 0750 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" "${starry_persist_root}/config"
sudo install -d -m 0750 -o root -g "${starry_control_gid}" \
  "${starry_persist_root}/control/secrets"
sudo install -d -m 0700 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" \
  "${starry_persist_root}/control/shared" \
  "${starry_persist_root}/control/state"
sudo install -m 0640 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" /dev/null \
  "${starry_persist_root}/config/config.yaml"

openssl rand -hex 32 | sudo tee \
  "${starry_persist_root}/control/shared/local-control.token" >/dev/null
sudo chown "${starry_control_uid}:${starry_control_gid}" \
  "${starry_persist_root}/control/shared/local-control.token"
sudo chmod 0600 \
  "${starry_persist_root}/control/shared/local-control.token"
```

Install the server certificate/key, client CA, and service public JWKS using
the exact filenames referenced by `control-agent.yaml` under
`control/secrets/`. Keep those files root-owned and readable only by the
selected control group. Install the HBBS-to-Kessoku CA and client identity
under `auth/secrets/`; HBBS mounts that directory read-only.

Use `/var/lib/starry-auth/jwks.json` as `connection_auth.jwks.file`. It maps to
`auth/cache/jwks.json` on the host and remains writable by root HBBS. Preserve
the generated `jwks.json.metadata.json` beside it during backup and restore.
Seed a valid JWKS before enabling `enforce`; never place the writable cache in
the read-only `auth/secrets/` directory.

Before enabling writes, make `config/`, `config/config.yaml`, and
`control/state/` owned by that numeric UID/GID. Group-write permission on a
file owned by a different UID is not enough for ownership-preserving atomic
replacement; a write-enabled Agent rejects that layout at startup. Keep
`control/state/` at mode `0700`, the managed config non-group/other-writable
(normally `0640`), and every parent component as a real directory. Keep the
Agent YAML and files under `control/secrets/` read-only.

Validate before startup:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
```

Then start HBBS/HBBR first, verify their existing data paths, and start the
Agent only during a controlled commissioning window. The sidecar shares the
HBBS network namespace, so its `127.0.0.1:21120` listener is host-loopback only
when HBBS uses Linux host networking. Edit the listener only for a
firewall-restricted private management interface; never expose it publicly.
