# rustdesk-server-starry documentation

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Home)

`rustdesk-server-starry` is an HBBS-only overlay for the official RustDesk
Server. It keeps the official source as the build base and adds ordered Geo
Relay selection, managed MMDB data, Secure TCP compatibility, and optional
WebSocket signalling.

## Understand the component boundary first

| Component | Role | Supplied or changed by Starry? |
| --- | --- | --- |
| Starry HBBS | Registers peers, coordinates connections, negotiates Secure TCP, evaluates Geo rules, and selects a Relay. | **Modified** by the overlay. |
| Official HBBR | Carries remote-control data when P2P is unavailable or a WebSocket endpoint is used. | **Not modified**. Use official HBBR or the unmodified upstream build bundled in Starry artifacts. |
| Account/API server | Handles login, address books, device data, and administration. | **Not included**. Select and secure it separately if needed. |
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
- WSS-to-WSS and WSS-to-native sessions through unmodified official HBBR.
- Certificate-verified `/ws/relay` health state for WSS and mixed allocation.
- Local management commands for reload, status, Relay listing, and rule tests.

## Choose your starting point

| Your situation | Start here |
| --- | --- |
| First deployment or limited Docker experience | [Getting Started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) |
| You arrived from the GHCR package page | [Docker Image Usage](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Image-Usage) |
| One Linux server, no separate Relay nodes | [Docker Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) |
| Existing systemd or Windows environment | [Native Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Native-Deployment) |
| One centre and several HBBR nodes | [Multi-Node Deployment](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment) |
| You need WebSocket | [Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS) |
| Server runs but a feature does not | [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) |
| You are changing a version | [Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) |

Docker Compose on a Linux host is the recommended deployment for most users.
It provides a repeatable service definition, visible persistent data, and a
clear rollback point.

## Recommended reading order

1. [Getting Started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started)
2. the page for your deployment method;
3. [Client Configuration](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration);
4. [Configuration Reference](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference);
5. [GEO Rules: Basics](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics);
6. [Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification); and
7. [Troubleshooting](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) when evidence points to a failure.

## Safe defaults

- Pin release versions for production.
- Keep the client Relay Server field empty when HBBS should allocate Geo
  Relays.
- Keep client WebSocket off unless the current network needs it.
- Do not enable WebSocket Signal until every configured Relay has a valid
  `/ws/relay` endpoint.
- Never bypass TLS verification.
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
