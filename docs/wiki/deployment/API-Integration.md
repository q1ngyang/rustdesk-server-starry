# Account/API integration

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration)

Starry does not include an account/API server. HBBS registration, HBBR Relay
traffic, account login, and the optional Starry Control Agent are separate
services. A working login therefore does not prove that signalling or Relay
traffic works, and a working Starry deployment does not provide accounts by
itself.

## Choose an API implementation

A compatible third-party RustDesk API implementation can be deployed with
Starry. Before using one, verify its supported RustDesk client versions,
server-key handling, database backup procedure, authentication model, and
whether its connection-token contract matches Starry connection
authentication.

For the most closely aligned integration, this project recommends the same
developer's
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku).
Use the [Kessoku Wiki](https://github.com/q1ngyang/rustdesk-api-kessoku/wiki)
for its installation, TLS, database, administrator, client, backup, and Starry
integration requirements.

The dedicated Kessoku + Starry joint-deployment page is still being prepared.
Until its link is added here, deploy and verify Starry with connection
authentication disabled, then follow the Kessoku Wiki for the API side. Do not
guess internal ports, paths, token claims, or reverse-proxy rules from Starry's
generic API example.

## Responsibility boundary

| Service | Provides | Does not provide |
| --- | --- | --- |
| Starry HBBS | ID registration, signalling, Secure TCP, Geo Relay selection, optional connection-token verification | Accounts, address books, web administration, or Relay data transfer |
| Starry-image HBBR | Native and WebSocket Relay data paths | Accounts, Geo policy, or token issuance |
| Account/API server | Login, account/device data, and the features documented by that API project | HBBS signalling or HBBR Relay transport |
| Starry Control Agent | Optional private management interface for one local HBBS | Public account API or RustDesk client login endpoint |

## Safe integration order

1. Complete [Getting Started](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started)
   and verify native plus Relay sessions without an API.
2. Deploy the API on its own hostname and persistent storage by following that
   project's documentation.
3. Verify HTTPS, administrator access, database backup, and RustDesk client
   login independently.
4. Set the RustDesk client's **API Server** field while keeping its **ID
   Server**, public key, and empty Relay field unchanged.
5. Repeat a real native session and a forced Relay session. Login success is
   not the acceptance test for either transport.
6. If the API explicitly supports Starry connection tokens, keep
   `connection_auth.mode: off` while provisioning trust material, move to
   `audit`, review evidence, and only then use `enforce`.
7. If the Control Agent is required, commission it read-only first and expose
   it only through the private mTLS management path described in the
   [Control Agent guide](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent).
8. Kessoku v3.0.6 can discover clients that are not signed in to its account
   API only when the center HBBS and Control Agent run patch-v1.2.2 or newer.
   The Agent verifies exact ID/UUID pairs; it does not export the registry.

## Reverse proxy and firewall

Use separate public hostnames for the ID/WSS endpoint and the account API when
the API project recommends it. Keep Starry's plain WebSocket backends
`21118/TCP` and `21119/TCP` private, and keep Control Agent `21120/TCP` on
loopback or a private management network.

The repository's `examples/nginx/api.example.conf` is a generic placeholder,
not a Kessoku deployment contract. Kessoku may separate public and internal
listeners and has its own trust requirements; its Wiki takes precedence for
all API proxy rules.

## Rollback

If API integration breaks client login, remove the client API Server setting
or restore the previous API deployment without changing the Starry identity
key. If connection authentication blocks sessions, return the mode to `audit`
or `off`, activate that valid configuration, and verify native and Relay
sessions again. Preserve API and Starry data backups separately.
