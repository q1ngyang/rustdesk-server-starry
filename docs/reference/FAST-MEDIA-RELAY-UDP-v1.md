# FastMedia Relay UDP v1

**English** | [简体中文](FAST-MEDIA-RELAY-UDP-v1.zh-CN.md)

This document defines Starry HBBR's additive `AKR1` UDP routing envelope for
Akari FastMediaV1. It does not replace the ordinary HBBR TCP/WebSocket stream.
Control, input, audio, clipboard, file transfer, negotiation, and fallback stay
on that reliable session. HBBR never receives an Akari media key and forwards
only the end-to-end encrypted `AKF1` payload.

The machine-readable wire contract is
[`contracts/fast-media/v1/akr1-wire.json`](../../contracts/fast-media/v1/akr1-wire.json).
The wire contract is `FROZEN` by the patch-v1.3.1 contract-candidate summary.
The runtime release remains `BLOCKED` until the recorded Akari end-to-end and
network-fault gates pass; wire immutability does not imply release approval.

## Header and messages

All integers are little-endian. Every datagram starts with this fixed 32-byte
header:

```text
0..4   "AKR1"
4      protocol = 1
5      kind: 1 Hello, 2 Cookie, 3 Bind, 4 Bound, 5 Media
6      role: 1 controller, 2 target
7      reserved = 0
8..16  non-zero FastMedia session_id (u64)
16..32 non-zero relay_allocation_id[16]
```

| Kind | Complete size | Body after byte 32 |
| --- | ---: | --- |
| Hello (1) | 56 | random nonce `[16]`, then eight zero bytes |
| Cookie (2) | 56 | echoed nonce `[16]`, cookie `[8]` |
| Bind (3) | 59..4154 | nonce `[16]`, cookie `[8]`, authorization length `u16`, 1..4096 signed bytes |
| Bound (4) | 32 | none |
| Media (5) | 121..authorized maximum | one complete encrypted AKF1 datagram |

HBBR accepts only Hello, Bind, and Media from clients. Cookie and Bound are
server responses. Unknown kinds, non-zero reserved bytes, zero IDs, truncated
messages, or messages above the authorization bound are dropped.

## Stateless cookie and amplification bound

Hello and Cookie have equal size. The eight-byte cookie authenticates source
IP and port, endpoint role, FastMedia session ID, Relay allocation ID, nonce,
and a short rotating epoch. HBBR accepts only the current or immediately
previous ten-second epoch. The cookie secret is process-local, so an HBBR
restart intentionally invalidates all cookies and in-memory bindings.

A source must obtain a new cookie after its address or port changes. HBBR does
not allocate session state for Hello and performs bounded global and per-IP
packet/byte admission before cookie work.

## Bind validation and state

Bind contains the role-specific Ed25519 combined authorization described in
[Fast Relay authorization v1](FAST-RELAY-AUTHORIZATION-v1.md). HBBR verifies the
cookie and signature, then requires all of these values to match its local
listener and outer header:

- authorization version 1, unexpired time, and no expiry more than 300 seconds
  in the future;
- `allow_fast_media_v1 = true` and `relay_udp_protocol = 1`;
- exact configured Relay identity and UDP port;
- exact non-zero 16-byte allocation ID and endpoint role;
- `relay_max_datagram` in `608..=1400` and source bitrate in
  `1000..=200000` Kbit/s;
- a non-empty session UUID within the server bound.

The first accepted Bind atomically pins one non-zero FastMedia session ID for
the allocation and one source tuple per signed role. A conflicting session,
role swap, Relay, allocation, or live tuple is rejected. HBBR sends Bound only
after the role is validly installed. Media forwarding begins only after both
controller and target are bound.

A previously bound role may rebind after an address/port change only with a
fresh Hello/Cookie/Bind using the same role grant. Successful rebind immediately
revokes the old tuple while preserving replay and rate state. Rebinds are
bounded to twelve per role per minute.

## AKF1 validation and forwarding

For every Media datagram, HBBR validates the cleartext AKF1 preamble without
decrypting the payload:

- magic `AKF1` and protocol version 1;
- non-zero sequence within a 128-packet replay window;
- inner session ID equals the AKR1 session ID;
- direction is 0 for outer controller role and 1 for outer target role;
- packet arrived from the currently pinned tuple and fits the signed datagram
  limit.

HBBR strips exactly the 32-byte AKR1 header and sends the complete remaining
AKF1 bytes unchanged to the opposite role. It does not parse media frames,
hold encryption keys, or terminate end-to-end confidentiality.

## Resource and traffic bounds

The encoded source ceiling in the grant is converted to a wire token bucket of
`ceil(max_bitrate_kbps × 1.45)`. Each role receives a burst no larger than
`max(256 KiB, 50 ms of its wire allowance)`. Per-role enforcement is combined
with bounded per-IP and global packet/byte budgets. Sustained excess is
dropped and counted; it never closes the reliable HBBR session.

Default lifecycle bounds are ten seconds for a half-bound allocation and
thirty seconds idle after binding, capped by the signed absolute expiry. The
allocation table, source-IP buckets, cleanup work per tick, authorization
bytes, and all exported counter dimensions are bounded. Reaching an allocation
or traffic limit rejects only new UDP work.

## Reliability and compatibility

FastMedia is an optional second data plane. UDP blocking, malformed grants,
binding timeout, rate limiting, HBBR UDP restart, packet loss, or listener
failure must leave FastCompat or the standard reliable media path alive. Akari
is responsible for falling back, retrying with bounded backoff, obtaining a new
cookie after migration, and automatically re-entering only after the UDP path
is authenticated again.

Official HBBR and patch-v1.3.0 HBBR do not advertise
`fast_media_relay_udp = 1`; HBBS never issues FastMedia grants for them. Older
Akari ignores or rejects the additive fields and remains on reliable Relay.

## HBBR configuration and telemetry

The enrolled Relay compatibility file may supply these non-secret settings:

```text
STARRY_RELAY_PUBLIC_ENDPOINT=relay.example.com:21117
STARRY_RELAY_FAST_MEDIA_UDP_PORT=21119
```

The HBBR process also accepts bounded allocation, half-bind/idle lifetime, and
per-IP/global packet/byte limits through `STARRY_RELAY_FAST_MEDIA_*`
environment variables. The existing HBBS public key verifies grants. Detailed
FastMedia capability, listener health, active allocations/streams, binds,
rebinds, forwarding, drops, rate limits, replay, expiry, and listener failures
are exported only in authenticated telemetry schema 2. Public Relay probe
responses contain no load or FastMedia runtime details.

## Release gate

Unit and real-HBBR process tests are necessary but not sufficient. This
contract must not become `FROZEN` until a real Akari↔HBBS↔HBBR integration test
proves both role grants and binds, encrypted forwarding, reliable fallback on
UDP failure, and automatic re-entry. Device/AP roaming, shaped-loss, bitrate,
and long-soak experiments remain separate release evidence.
