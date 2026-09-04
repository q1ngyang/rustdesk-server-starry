# rustdesk-server-starry documentation

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Home)

`rustdesk-server-starry` extends the official RustDesk Server HBBS while
keeping the upstream source as its pinned build base. It adds ordered Geo
Relay selection, managed MMDB data, Secure TCP, optional WebSocket signalling,
connection authentication, safe configuration activation, Relay simulation,
and an optional least-privilege Linux Control Agent. The same pinned image
contains HBBR's reliable upstream path plus optional role-authorized AKR1 UDP,
bounded Akari probes, and authenticated telemetry.

## Understand the component boundary first

| Component | Role | Supplied or changed by Starry? |
| --- | --- | --- |
| Starry HBBS | Registers peers, coordinates connections, negotiates Secure TCP, evaluates Geo rules, and selects a Relay. | **Modified** by the overlay. |
| Starry-image HBBR | Carries reliable Relay data and optionally routes encrypted FastMedia datagrams. | The upstream TCP/WS byte path stays reliable. Starry adds bounded quality probes, authenticated telemetry, and separately supervised AKR1 UDP. |
| Starry Control Agent | Exposes a fixed management API for one local HBBS over mTLS and scoped service JWTs. | Optional Linux component. Configuration writes are disabled by default. |
| Account/API server | Handles login, address books, device data, and administration. | **Not included**. A compatible third-party API can be used; Kessoku is the recommended integration. |
| RustDesk client | Registers with HBBS and establishes P2P or Relay sessions. | Not included. |

An API login does not prove that the HBBS signalling transport works. An HBBS
registration does not prove that HBBR data can flow. The documentation keeps
these layers separate so that deployment and diagnosis stay evidence-based.

## What Starry adds

- Strictly ordered Relay selection using facts about both public client
  addresses.
- Country, continent, subdivision, city, GeoNames ID, ASN, and ISP matching.
- Scheduled MMDB download with validation and last-known-good retention.
- Client-compatible Secure TCP on native HBBS `21116/TCP`.
- Optional persistent `/ws/id` signalling for constrained networks.
- WSS-to-WSS and WSS-to-native sessions through the bundled HBBR.
- Opt-in Akari candidate probing and dual-end RTT/jitter/loss/load scoring while official clients keep one legacy Relay.
- Default-off signed FastCompat/FastMedia authorization for the exact Relay
  selected by HBBS; FastMedia uses role grants, bounded active-session
  renewal, and keeps the reliable path.
- Generation-safe Akari Profile activation with a matching Ready ACK, an opaque
  route lease, explicit current-route deactivation, and bounded verified rapid
  re-registration; official clients retain their existing registration path.
- Certificate-verified `/ws/relay` health state for WSS and mixed allocation.
- Last-known-good config generation/digests and synchronous activation ack.
- Strict optional connection JWT audit/enforcement across native TCP, Secure
  TCP, and WSS, with UDP initiation unsupported.
- Immutable Relay snapshots and side-effect-free allocation simulation.
- A loopback local protocol and optional mTLS/RBAC Control Agent.
- Optional SP1 Control-Agent/Relay bootstrap, bounded enrollment, persistent
  identities, and schema-v5 downgrade preview/export.

## Choose your starting point

| Your situation | Start here |
| --- | --- |
| First deployment or limited Docker experience | [Getting Started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) |
| You arrived from the GHCR package page | [Docker Image Usage](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Image-Usage) |
| One Linux server, no separate Relay nodes | [Docker Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) |
| Existing systemd or Windows environment | [Native Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Native-Deployment) |
| One centre and several HBBR nodes | [Multi-Node Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment) |
| You need WebSocket | [Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS) |
| You need accounts, login, or an API | [Account/API Integration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration) |
| You are integrating login with HBBS connection authorization | [Connection Authentication](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Connection-Authentication) |
| You need Relay visibility or managed config transactions | [Control Agent](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent) |
| Server runs but a feature does not | [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) |
| You are changing a version | [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) |

Docker Compose on a Linux host is the recommended deployment for most users.
It provides a repeatable service definition, visible persistent data, and a
clear rollback point.

## Recommended reading order

1. [Getting Started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started)
2. the page for your deployment method;
3. [Client Configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration);
4. [Account/API Integration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration), when accounts are required;
5. [Configuration Reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference);
6. [Connection Authentication](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Connection-Authentication) or [Control Agent](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent) when those optional features are in scope;
7. [GEO Rules: Basics](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics);
8. [Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification); and
9. [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) when evidence points to a failure.

## Safe defaults

- Pin release versions for production.
- Keep the client Relay Server field empty when HBBS should allocate Geo
  Relays.
- Keep client WebSocket off unless the current network needs it.
- Do not enable WebSocket Signal until every configured Relay has a valid
  `/ws/relay` endpoint.
- Never bypass TLS verification.
- Keep connection authentication off until audit evidence is complete; do not
  treat `audit` as enforcement.
- Commission the Control Agent read-only and keep HBBS `21115` on loopback.
- Distribute only `id_ed25519.pub`; keep `id_ed25519` private and backed up.
- Treat Compose validation, an open port, or HTTP 101 as partial evidence, not
  as a successful desktop-control session.

## Project and legal status

This is an unofficial community project. It is not affiliated with RustDesk,
MaxMind, any MMDB mirror provider, or any AI service provider. No GeoLite2
database is built into the image. Parts of the code and documentation were
generated or revised with AI assistance and carry no additional warranty.

Source: <https://github.com/q1ngyang/rustdesk-server-starry>

Licence: AGPL-3.0
