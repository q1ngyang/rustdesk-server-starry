# Relay Telemetry Security and Operations

## Deployment

Create one random 32-byte-or-longer secret per trust domain and mount it read-only at the same absolute path in HBBR and HBBS. Configure HBBR with `STARRY_RELAY_TELEMETRY_SECRET_FILE` and each HBBS `relay_health` entry with an internal `wss://.../ws/telemetry` URL plus `telemetry_secret_file`. The YAML contains only the path. Do not place the value in environment variables, proxy query strings, access logs, or Control API documents.

Restrict `/ws/telemetry` to HBBS source networks at the firewall/reverse proxy and use internal mTLS where available. Preserve the three `x-starry-telemetry-*` request headers and two response headers through the proxy; disable caching. Keep `/ws/relay` public for official clients and Akari probes. A normal WSS client receives capability metadata but no load details.

Recommended HBBR settings:

```text
STARRY_RELAY_TELEMETRY_SECRET_FILE=/run/secrets/starry-relay-telemetry
STARRY_RELAY_MAX_SESSIONS=10000
STARRY_RELAY_PROBE_PER_IP_PER_MINUTE=120
STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE=10000
STARRY_RELAY_DRAINING_FILE=/run/starry/hbbr.draining
STARRY_RELAY_PUBLIC_ENDPOINT=relay.example.com:21117
STARRY_RELAY_FAST_MEDIA_UDP_PORT=21119
```

Rotate a secret by temporarily accepting the old trust domain as legacy fallback, rolling HBBR with the new mounted file, then rolling/reloading HBBS endpoints. Because v1 has one active HMAC key, an in-place file replacement without coordinated process reload can cause a short fail-closed telemetry gap.

## Alerting versus diagnosis

Alert on sustained conditions, not a single scrape:

- Relay `state != healthy`, `stale=true`, or missing authenticated telemetry for more than two health intervals;
- `draining=true` outside a maintenance window;
- active sessions at capacity with a rising `admission_rejections` counter;
- rising `telemetry_auth_failures`, `reports_invalid`, `reports_binding_mismatch`, or `reports_late`;
- a sudden rise in fallback rate, especially `no_reachable_candidate`;
- sustained `stage_timeouts` or `probe_failure` fallbacks after normalizing by
  `primary_probes`;
- repeated process-instance changes or a non-monotonic sequence error.

Use these primarily for diagnosis/capacity planning: instantaneous active/pending counts, bandwidth EMA, load basis points, successful/malformed/unsupported probe counts, per-Relay selection counts, cache hits, and hysteresis decisions. `probe_rate_limited` may become an alert only when correlated with client failures or an unexpected source distribution.

Adaptive-flow counters are also diagnostic unless correlated with failures:
`primary_accepted / primary_probes` describes how well GEO ordering performs,
`expansions_triggered` and `expanded_decisions` describe how often quality has
to override the primary, and `estimated_probe_attempts_saved` measures the
work avoided by staging. `p2p_cancellations` is normally healthy and must not
be alerted on by itself.

For telemetry schema 2, alert when an enabled FastMedia listener stays
unhealthy for more than two intervals, `listener_failures` continues rising,
or a material active workload has sustained `rate_limited`, `replay_rejected`,
or `grant_rejected` growth. Alert when HBBS configuration advertises a UDP port
but fresh telemetry reports another port. Treat `hello_accepted`, individual
cookie/bind failures, rebinds, forwarded packet/byte totals, active
allocations/streams, and occasional drops as diagnostic or capacity-planning
signals unless correlated with fallback or user-visible failure. A rebind rise
during an AP migration test is expected; a listener restart is not proof that
the reliable HBBR session failed.

All Control dimensions are bounded. Offer/fallback reasons are fixed fields, and per-Relay selections contain at most 256 configured Relay keys plus an overflow counter. The API never returns client IPs, session/allocation identifiers, nonces, or raw reports.

## Rolling upgrade and rollback

patch-v1.3.1 upgrade order:

1. Provision TLS/mTLS policy and secret files without changing traffic.
2. Roll HBBR patch-v1.3.1 while FastMedia policy remains disabled. Existing
   reliable sessions and official clients remain compatible. Verify public
   probes contain no load, authenticated telemetry is schema 2, instance and
   sequence are stable, and the UDP listener is healthy where configured.
3. Roll HBBS patch-v1.3.1 with schema v4 or both Fast switches false. Verify
   fresh typed telemetry and ordinary Native/WSS/mixed Relay first.
4. Roll Control Agents and validate the schema-v5/OpenAPI fixtures. They read
   HBBS local control and never scrape HBBR directly.
5. Canary FastCompat, then FastMedia on an allowlisted Relay and Akari pair.
   Keep official-client and reliable-fallback tests in the gate.

For a v1.3.1→v1.3.0 rollback, disable FastMedia first and wait for active
authorizations, allocations, streams, and the last grant expiry to drain. Use
the schema-v4 downgrade preview/export and require at least ninety days on
Agent/Relay certificates. Roll HBBS back with the compatible schema-v4 file,
then HBBR with `relay-compat.env`, then Control Agent. Ordinary
`relay_server`, native TCP, WS/WSS pairing, schema-1 telemetry, and official
clients remain available. Preserve and do not let old binaries rewrite the
pairing/enrollment state.
