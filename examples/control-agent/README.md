# Control Agent Compose example

**English** | [简体中文](README.zh-CN.md)

This example is an opt-in Linux sidecar deployment. Read the full
[Control Agent guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent)
before starting it. The checked-in configuration is read-only
(`write_enabled: false`) and binds the mTLS API to host loopback.

Prepare paths without placing real secrets in the repository:

```sh
cp .env.example .env
mkdir -p data/hbbs data/starry data/control secrets
touch data/starry/config.yaml
chmod 0700 data/control
chmod 0640 data/starry/config.yaml
```

Install the server certificate/key, client CA, and service public JWKS using
the exact filenames referenced by `control-agent.yaml`. Make `data/starry`,
`data/control`, the Agent YAML, and the secret files accessible to the numeric
UID/GID selected in `.env`; keep private keys unreadable by other users. Do not
make a secret world-writable.

Before enabling writes, make `data/starry`, `data/starry/config.yaml`, and
`data/control` owned by that numeric UID/GID. Group-write permission on a file
owned by a
different UID is not enough for ownership-preserving atomic replacement; a
write-enabled Agent rejects that layout at startup. Keep `data/control` at
mode `0700`, the managed config non-group/other-writable (normally `0640`),
and every parent component as a real directory. Keep the Agent YAML and
files under `secrets/` read-only.

Validate before startup:

```sh
docker compose --env-file .env -f compose.yaml config --quiet
```

Then start HBBS/HBBR first, verify their existing data paths, and start the
Agent only during a controlled commissioning window. The sidecar shares the
HBBS network namespace, so its `127.0.0.1:21120` listener is host-loopback only
when HBBS uses Linux host networking. Edit the listener only for a
firewall-restricted private management interface; never expose it publicly.
