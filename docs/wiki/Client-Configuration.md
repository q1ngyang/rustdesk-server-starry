# Client configuration

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration)

Configure both endpoints against the same centre identity. A server-side
deployment is not complete until real clients register and transfer a desktop
session.

## Required fields

Open the RustDesk client **Settings → Network**, unlock the settings, and use:

| Client field | Value |
| --- | --- |
| ID Server | Public HBBS hostname, for example `id.example.com`, or an explicit `:21116` address when non-default. |
| Key | Complete one-line content of the centre `id_ed25519.pub`. This is not an API token or licence key. |
| Relay Server | Leave empty when Starry HBBS should dynamically allocate a Geo Relay. |
| API Server | Optional separate HTTPS URL, such as `https://api.example.com`, only when an API is deployed. |
| Use WebSocket | Off by default; enable per client only when that network needs WSS. |

The
[official RustDesk client configuration guide](https://rustdesk.com/docs/en/self-host/client-configuration/)
also distinguishes ID, Relay, API, and public Key fields. Starry does not
change their meaning.

## Why Relay Server should stay empty

A non-empty static Relay field tells the client to use that address and can
bypass the dynamic Relay address returned by HBBS. Leave it empty to exercise
Starry rules and failover.

If you temporarily set a Relay for diagnosis, remove it before validating Geo
allocation.

## WebSocket combinations

| Client A | Client B | Signalling/data expectation |
| --- | --- | --- |
| Off | Off | Native signalling. P2P preferred; native HBBR only when required. |
| On | Off | A uses WSS, B remains native. The session is Relay-only through one HBBR using mixed transports. |
| Off | On | Same as above with directions reversed. |
| On | On | WSS signalling and WSS Relay; no normal P2P path. |

The server setting `websocket_signal.enabled` permits the path but never
changes a client switch. Conversely, a client switch cannot compensate for a
missing `/ws/id`, `/ws/relay`, or invalid certificate.

## API login

An API is optional for base self-hosted control and is not supplied by Starry.
When used:

1. verify the HTTPS API status and login independently;
2. confirm the API exposes the same HBBS public key and ID Server expected by
   the clients;
3. leave Relay Server empty for Geo allocation; and
4. repeat the remote-control test while logged in, because authenticated
   clients may negotiate Secure TCP on `21116/TCP` before punch-hole or Relay.

Successful API login is not evidence that Secure TCP, HBBR, or the desktop data
path works.

## First acceptance pair

Use two known devices and record:

- client versions and platforms;
- both configured ID/API/Key values, with secrets redacted;
- public egress addresses as observed by HBBS;
- whether each WebSocket switch is on;
- whether the final session is P2P or Relay;
- selected Relay hostname and shared Relay UUID; and
- timestamps for matching client/server logs.

Test native first, then API login, then WSS. Change one dimension at a time.

## Common mistakes

- Copying `id_ed25519` instead of the public `.pub` content.
- Treating an API access token or a commercial licence key as the server Key.
- Leaving an old static Relay address in one client.
- Enabling WebSocket on only the client while the server paths are incomplete.
- Assuming WSS should preserve normal P2P; it is intentionally Relay-only.
- Testing two clients behind one NAT without using the same observed public IP
  twice in `test-geo`.
- Comparing logs from different attempts instead of one timestamped session.

Continue with
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).
