# rustdesk-server-starry

**English** | [简体中文](docs/project/README.zh-CN.md)

## Project overview

`rustdesk-server-starry` is an HBBS-focused overlay for the official
[`rustdesk/rustdesk-server`](https://github.com/rustdesk/rustdesk-server).
It adds policy-driven Relay selection and authenticated-client transport
compatibility without maintaining a permanently diverged copy of the upstream
source tree.

Every build starts from an exact official RustDesk Server revision. The
[`scripts/apply_overlay.py`](scripts/apply_overlay.py) script verifies fixed
source anchors, injects the Starry modules, and must produce no further changes
when applied a second time. If an upstream structural change invalidates an
anchor or a test, publication stops instead of producing a partially patched
server.

Starry extends **HBBS** with:

- ordered Relay selection using both endpoints' country, city, subdivision,
  ASN, and ISP facts;
- validated MMDB download, replacement, retention, and scheduled update;
- RustDesk-client-compatible Secure TCP negotiation and encrypted transport on
  native HBBS TCP `21116`;
- optional persistent WebSocket signalling on `/ws/id`, including WSS-to-WSS
  and WSS-to-native Relay sessions; and
- certificate-verified `/ws/relay` health filtering;
- Akari-only candidate Relay active probing with dual-end RTT/jitter/loss and
  trusted HBBR load scoring, prefix-cache hysteresis, and a legacy single-Relay fallback;
- default-off, short-lived Ed25519 authorization for Akari FastCompat and
  role-bound FastMediaV1 after exact auth allow and the final server-selected
  Relay, with a separately supervised HBBR AKR1 UDP data plane;
- generation-safe Profile Activation Leases for Akari, including a matching
  Ready ACK, explicit current-lease deactivation, bounded verified rapid
  re-registration, and official-client-compatible defaults;
- schema v5 last-known-good activation with generation, digest, and synchronous
  subsystem acknowledgements;
- optional strict Ed25519 connection JWT audit/enforcement for both
  `PunchHoleRequest` and direct `RequestRelay` on native TCP, Secure TCP, and
  WSS, with UDP initiation explicitly unsupported; and
- SP1 Control-Agent/Relay bootstrap, bounded Relay enrollment, persistent
  identities, and a no-side-effect v5-to-v4 downgrade export; and
- immutable Relay snapshots, side-effect-free allocation simulation, plus a
  separate Linux Control Agent protected by mTLS, scoped service JWTs, and
  atomic configuration transactions.

The project deliberately keeps the component boundary narrow:

| Component | Starry scope | Deployment responsibility |
| --- | --- | --- |
| HBBS | Modified by the overlay | Run the Starry `hbbs` binary or image command. |
| HBBR | Reliable Relay plus optional AKR1 UDP | Keeps the upstream TCP/WS byte-forwarding path, answers public quality probes without load, exposes detailed state only through authenticated telemetry, and optionally routes encrypted Akari AKF1 datagrams after role-bound grants. |
| Control Agent | Optional Starry component; Linux only in v1.3 | Keep HBBS local control on loopback. Expose the Agent only on a private management path with mTLS and scoped service JWTs. Configuration writes are disabled by default. |
| Account/API server | Not included | Account login, address books, device data, and administration require a separately selected API implementation. |

An account API login, the HBBS signalling connection, the optional Starry
Control API, and the HBBR data path are separate protocol layers. Starry does
not turn a third-party API into a Relay and does not replace the RustDesk
client. The optional FastMedia UDP listener is independent from the ordinary
HBBR stream, which always remains the reliable fallback.

Current development release: **patch-v1.3.1 (release candidate blocked)**. See
the [`patch-v1.3.1` release notes](docs/releases/RELEASE-NOTES-patch-v1.3.1.md) and
[`changelog`](docs/releases/CHANGELOG.md). Docker images are published at
[`ghcr.io/q1ngyang/rustdesk-server-starry`](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry).

The patch-v1.3.1 candidate matrix is Docker `linux/amd64` plus Linux
x86_64 binaries and amd64 DEB packages. ARM is best-effort source
compatibility; Windows is an experimental non-blocking build. Neither is a
promised v1.3.1 artifact. Publication remains blocked by the end-to-end
FastMedia fallback/re-entry and full release gates.

> This is an unofficial community project and is not affiliated with or
> endorsed by RustDesk, MaxMind, any MMDB mirror provider, or any AI service
> provider. The image does not include a GeoLite2 database; operators must
> select lawful, trusted data sources and comply with their licences. Parts of
> the code and documentation were generated or revised with AI assistance.
> They remain subject to the same project licence and are provided without any
> additional warranty.

## Documentation

English is the default documentation language. Every narrative guide has a
Simplified Chinese counterpart. Copy-ready configuration and orchestration
files are shared by both languages so that executable examples cannot drift.

The [classified documentation index](docs/README.md) contains the local
sources, container manual, release notes, and technical references.

| Guide | Purpose |
| --- | --- |
| [Getting started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) | Complete single-host Docker walkthrough from DNS and firewall to Geo, WSS, and real-client verification. |
| [Docker image usage](docs/container/CONTAINER.md) | Pull, inspect, run, pin, upgrade, and troubleshoot the GHCR image. |
| [Docker deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) | Recommended single-host Docker Compose deployment. |
| [Native deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Native-Deployment) | Supported amd64 DEB/Linux deployment and non-blocking compatibility notes. |
| [Multi-node deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment) | Centre HBBS, version-locked Starry-image HBBR nodes, and optional account services. |
| [Reverse proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS) | Exact `/ws/id`, `/ws/relay`, API, certificate, and firewall requirements. |
| [Client configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration) | ID Server, API Server, public key, Relay field, and WebSocket switch. |
| [Account/API integration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration) | Third-party compatibility, recommended Kessoku integration, responsibility boundaries, and safe rollout. |
| [Configuration reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference) | Every schema field, default, valid range, dependency, and fallback. |
| [Connection authentication](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Connection-Authentication) | JWT profile, audit-to-enforce rollout, transport coverage, failure handling, and rollback. |
| [Control Agent](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent) | Linux deployment, mTLS/service-JWT authorization, read-only mode, transactions, recovery, and API contracts. |
| [Profile Activation Lease v1](docs/reference/PROFILE-ACTIVATION-LEASE-v1.md) | Akari matching-ACK switch rule, current-route deactivation, multi-node leases, observability, rollout, and rollback. |
| [Fast Relay authorization v1](docs/reference/FAST-RELAY-AUTHORIZATION-v1.md) | Server-selected Relay grants, additive fields 1–12, compatibility, privacy, and issuance gates. |
| [FastMedia Relay UDP v1](docs/reference/FAST-MEDIA-RELAY-UDP-v1.md) | AKR1 cookie/bind/forward/rebind wire and resource contract. |
| [Starry Pairing v1](docs/reference/STARRY-PAIRING-v1.md) | SP1 Control Agent/Relay bootstrap, persistence, manual compatibility, and downgrade. |
| [GEO rules: basics](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics) | Country rules, priority, symmetry, fallback, and `test-geo`. |
| [GEO rules: advanced](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Advanced) | City, ASN, ISP, nested expressions, quoting, and design patterns. |
| [Operations and verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification) | Layered checks from static validation to real desktop sessions. |
| [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) | Diagnose configuration, MMDB, Secure TCP, WSS, HBBR, API, and upgrade failures. |
| [Upgrade and rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) | Back up, migrate schema, verify a release, and restore safely. |
| [Architecture and build](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Architecture-and-Build) | Overlay mechanics, protocol boundaries, tests, and release automation. |
| [简体中文文档](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Home) | 中文文档主页。 |

## Licence

The official RustDesk Server source and the Starry overlay are distributed
under the GNU Affero General Public License v3.0. Published binaries and images
are built from the corresponding pinned upstream revision plus this overlay;
see [`LICENSE`](LICENSE).
