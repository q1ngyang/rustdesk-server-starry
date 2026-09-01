# Relay Telemetry v1

Relay Telemetry v1 separates the public client probe from operational load data. HBBR never connects to Kessoku or the Control API. HBBS pulls, authenticates, validates, and ages telemetry; the Control Agent exposes only bounded aggregates from HBBS.

## Public probe

`/ws/relay` remains compatible with official WebSocket Relay clients. Its upgrade response may contain only `x-starry-version`, `x-starry-relay-probe-protocol`, and `x-starry-relay-load-protocol`. `RelayProbeResponse` returns the requested nonce, protocol versions, capabilities, and optional Starry version. Its `load` field is absent. Native TCP and WS/WSS use the same response contract.

HBBR applies fixed-window global and transport-source-IP limits to every parsed `RelayProbeRequest` before nonce/version validation. Defaults are 10,000 global and 120 per IP per minute; `STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE` and `STARRY_RELAY_PROBE_PER_IP_PER_MINUTE` accept 1..1,000,000. The IP table is capped at 4,096 entries. Only aggregate malformed, unsupported, rate-limited, and successful counters are retained. Nonces and raw messages are never logged or retained as metrics.

## Authenticated telemetry transport

Production deployments MUST protect `/ws/telemetry` with TLS. Internal mTLS at the reverse proxy is preferred. When TLS terminates at that proxy, Starry additionally requires HMAC-SHA-512/256 using a secret file shared by HBBR and HBBS:

- HBBR: `STARRY_RELAY_TELEMETRY_SECRET_FILE=/run/secrets/starry-relay-telemetry`
- HBBS endpoint: `telemetry_secret_file: /run/secrets/starry-relay-telemetry`

The file contains 32..1,024 bytes. A trailing CR/LF is removed and SHA-256 derives the fixed HMAC key. The secret value MUST NOT appear in YAML, a URL, logs, or Control API output.

HBBS sends:

- `x-starry-telemetry-timestamp`: Unix seconds, accepted within ±30 seconds;
- `x-starry-telemetry-nonce`: 32 hexadecimal characters;
- `x-starry-telemetry-auth`: lowercase hexadecimal HMAC over `starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry`.

HBBR keeps a 4,096-entry, 30-second replay cache. On success it returns base64url-no-pad JSON in `x-starry-telemetry` and a hexadecimal HMAC over `starry-telemetry-response-v1\n{request_nonce}\n{encoded_payload}` in `x-starry-telemetry-auth`. Missing, malformed, expired, replayed, or invalid authentication receives HTTP 401 without telemetry headers.

## Metric semantics

- `active_sessions` counts only authenticated Relay requests whose two legs have paired and acquired admission. It decrements when forwarding ends.
- `pending_pairs` counts unpaired first legs. A per-entry generation prevents an old timeout from removing a newer waiter with the same UUID.
- `bandwidth_bps` is the sum of per-session EMAs in bit/s. Samples cover at least one second and use α=0.25 (`bandwidth_ema_alpha_basis_points=2500`).
- `capacity_sessions` is the enforced `STARRY_RELAY_MAX_SESSIONS` limit, not a hint. `capacity_bandwidth_bps` is configured HBBR total bandwidth.
- `load_basis_points` is the larger of session and bandwidth utilization, capped at 10,000.
- `draining` is set by `STARRY_RELAY_DRAINING` or the existence of `STARRY_RELAY_DRAINING_FILE`. Draining and capacity reject new pairings without terminating existing sessions.
- `admission_rejections` counts capacity, draining, and bounded-pending rejection events.

Every payload contains schema version 1, process instance UUID, monotonic sequence, observation timestamp, uptime, version, capability versions, lifecycle gauges, and aggregate counters. HBBS rejects non-monotonic sequence/uptime for one instance. A changed instance is accepted as a restart and counted. Future observations beyond 30 seconds fail validation; old observations remain visible but become stale under `max_telemetry_age_seconds` and are excluded from quality scoring.

Legacy/official HBBR may still be checked through `/ws/relay`. Its detailed fields remain `null`, it cannot become a quality candidate, and it remains eligible for ordinary `relay_server` fallback.
