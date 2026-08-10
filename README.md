# rustdesk-server-starry

**English** | [简体中文](README.zh-CN.md)

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
- certificate-verified `/ws/relay` health filtering plus local inspection and
  rule-testing commands.

The project deliberately keeps the component boundary narrow:

| Component | Starry scope | Deployment responsibility |
| --- | --- | --- |
| HBBS | Modified by the overlay | Run the Starry `hbbs` binary or image command. |
| HBBR | Not modified | Use official HBBR. Starry release artifacts may bundle an unmodified HBBR built from the same pinned upstream revision for convenience. |
| Account/API server | Not included | Account login, address books, device data, and administration require a separately selected API implementation. |

An API login, the HBBS signalling connection, and the HBBR data path are
separate protocol layers. Starry does not turn a third-party API into a Relay,
does not fork HBBR, and does not replace the RustDesk client.

Current overlay version: **patch-v1.1.0**. See the
[`patch-v1.1.0` release notes](RELEASE-NOTES-patch-v1.1.0.md) and
[`changelog`](CHANGELOG.md). Docker images are published at
[`ghcr.io/q1ngyang/rustdesk-server-starry`](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry).

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

| Guide | Purpose |
| --- | --- |
| [Getting started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) | Choose a deployment and reach the first verified client connection. |
| [Docker image usage](CONTAINER.md) | Pull, inspect, run, pin, upgrade, and troubleshoot the GHCR image. |
| [Docker deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) | Recommended single-host Docker Compose deployment. |
| [Native deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Native-Deployment) | DEB, Linux binary, systemd, and Windows deployment. |
| [Multi-node deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment) | Centre HBBS, official HBBR nodes, and an optional third-party API. |
| [Reverse proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS) | Exact `/ws/id`, `/ws/relay`, API, certificate, and firewall requirements. |
| [Client configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration) | ID Server, API Server, public key, Relay field, and WebSocket switch. |
| [Configuration reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference) | Every schema field, default, valid range, dependency, and fallback. |
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
