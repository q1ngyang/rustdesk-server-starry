# Documentation index

**English** | [简体中文](README.zh-CN.md)

All maintained Markdown documentation lives under `docs/`, apart from the
root [project README](../README.md), which remains the GitHub landing page.
Only `docs/wiki/` contains online Wiki page sources; the other categories hold
repository documentation.
This index opens the source files in the repository; the
[online Wiki](https://github.com/q1ngyang/rustdesk-server-starry/wiki) provides
the published reading experience.

## Start here

1. Read the [project overview](wiki/Home.md).
2. Follow [Getting Started](wiki/getting-started/Getting-Started.md) for a
   complete single-host Docker deployment without an account/API service.
3. Configure the [clients](wiki/getting-started/Client-Configuration.md), then
   run the [verification checklist](wiki/operations/Operations-and-Verification.md).

## Guides by task

| Category | Local documents |
| --- | --- |
| Introduction and first deployment | [Overview](wiki/Home.md), [Getting Started](wiki/getting-started/Getting-Started.md), [Client Configuration](wiki/getting-started/Client-Configuration.md) |
| Deployment | [Docker](wiki/deployment/Docker-Deployment.md), [Native](wiki/deployment/Native-Deployment.md), [Multi-Node](wiki/deployment/Multi-Node-Deployment.md), [Reverse Proxy and TLS](wiki/deployment/Reverse-Proxy-and-TLS.md), [Account/API Integration](wiki/deployment/API-Integration.md) |
| GHCR image | [Standalone container manual](container/CONTAINER.md), [Wiki image guide](wiki/deployment/Docker-Image-Usage.md) |
| Configuration and features | [Parameter Reference](wiki/configuration/Configuration-Reference.md), [GEO Basics](wiki/configuration/GEO-Rules-Basics.md), [Advanced GEO Rules](wiki/configuration/GEO-Rules-Advanced.md), [Connection Authentication](wiki/configuration/Connection-Authentication.md), [Control Agent](wiki/configuration/Control-Agent.md) |
| Operations | [Verification](wiki/operations/Operations-and-Verification.md), [Relay Telemetry Security and Operations](wiki/Relay-Telemetry-Operations.md), [Troubleshooting](wiki/operations/Troubleshooting.md), [Upgrade and Rollback](wiki/operations/Upgrade-and-Rollback.md) |
| Releases | [Changelog](releases/CHANGELOG.md), [patch-v1.3.2 preview](releases/RELEASE-NOTES-patch-v1.3.2.md), [patch-v1.3.1](releases/RELEASE-NOTES-patch-v1.3.1.md), [patch-v1.3.0](releases/RELEASE-NOTES-patch-v1.3.0.md) |
| Technical reference | [Architecture and Build](wiki/reference/Architecture-and-Build.md), [Relay Quality Protocol v1](reference/RELAY-QUALITY-PROTOCOL-v1.md), [Relay Telemetry v3](reference/RELAY-TELEMETRY-v3.md), [Fast Relay Authorization v1](reference/FAST-RELAY-AUTHORIZATION-v1.md), [FastMedia Relay UDP v1](reference/FAST-MEDIA-RELAY-UDP-v1.md), [FastMedia Renewal v1](reference/FAST-MEDIA-RENEWAL-v1.md), [Starry Pairing v1](reference/STARRY-PAIRING-v1.md), [Profile Activation Lease v1](reference/PROFILE-ACTIVATION-LEASE-v1.md), [Authentication Profile](reference/auth/v1/profile.md), [Client Compatibility](reference/auth/v1/client-compatibility.md) |
| Example walkthroughs | [Control Agent Compose](examples/control-agent/README.md); runnable files stay in the root [examples directory](../examples) |
| Project maintenance | [Publication and metadata](project/PROJECT-METADATA.md), [Chinese project README](project/README.zh-CN.md) |
| Historical records | [WebSocket development plan](archive/PATCH-V1.1.0-WEBSOCKET-DEVELOPMENT-PLAN.md) (original Chinese record, not current deployment guidance) |

## Directory layout

```text
docs/
├── README.md / README.zh-CN.md   # Local index
├── wiki/                       # Sources exported to the online Wiki
│   ├── Home.md / ZH-CN-Home.md / _Sidebar.md
│   ├── getting-started/
│   ├── deployment/
│   ├── configuration/
│   ├── operations/
│   └── reference/
├── container/                  # Independent GHCR image manual
├── releases/                   # Changelog and versioned patch notes
├── reference/                  # Relay-quality, Fast Relay/Media, Pairing, activation, and authentication contracts
├── examples/control-agent/    # Example instructions, not runnable files
├── project/                    # Chinese project README and publication notes
└── archive/                    # Historical development records
```

## Editing and publication rules

- Edit the source in its category; do not leave duplicate guides in the root,
  `.github`, `contracts`, or runnable example directories.
- Keep each Wiki page's filename unchanged when moving it between categories.
  Publication flattens `docs/wiki/` to the existing filenames, so online Wiki URLs
  do not change. Filenames must be unique across all categories.
- Wiki translations use `ZH-CN-*.md`; other maintained guides use
  `*.zh-CN.md`. The root README's translation is the intentional exception at
  `project/README.zh-CN.md`. Historical records retain their original language.
- Configuration, Compose/Nginx examples, schemas, fixtures, and build scripts
  stay at their existing executable paths; only explanatory Markdown moves.
- Ignored local development plans remain private, even when stored in
  `archive/`. Neither the documentation checks nor the Wiki/Release exporter
  includes them.
- Existing release-tag links are historical references and are not rewritten.
  New Release documentation keeps its familiar download filenames, while its
  source links and new image documentation metadata point to the build commit.

From the repository root, validate before review:

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts -p 'test_docs.py'
python3 scripts/check_workflows.py
```

Exporting is local-only. Review the [publication procedure](project/PROJECT-METADATA.md#documentation-export-and-publication)
and obtain approval before pushing repository changes or updating the Wiki.
