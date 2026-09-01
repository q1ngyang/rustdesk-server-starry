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

All Control dimensions are bounded. Offer/fallback reasons are fixed fields, and per-Relay selections contain at most 256 configured Relay keys plus an overflow counter. The API never returns client IPs, session/allocation identifiers, nonces, or raw reports.

## Rolling upgrade and rollback

Upgrade order:

1. Provision TLS/mTLS policy and secret files without changing traffic.
2. Roll HBBR patch-v1.3.0. Existing data sessions and official clients remain compatible; verify public probes contain no load and authenticated telemetry reports schema 1.
3. Roll HBBS patch-v1.3.0 with quality disabled or the affected Relay listed as explicit legacy fallback. Verify fresh telemetry, stable instance/sequence, and Control inventory.
4. Enable quality candidates gradually, then remove temporary legacy declarations.
5. Roll Control Agents last; they read HBBS local control only and never contact HBBR.

For rollback, disable `relay_quality` first. Restore HBBS endpoints to `/ws/relay` and remove `telemetry_secret_file` before rolling HBBS back. HBBR can then be rolled back independently; ordinary `relay_server`, native TCP, WS/WSS pairing, and official-client behavior remain available throughout. Do not leave a schema-v4 telemetry field in configuration consumed by an older HBBS.
