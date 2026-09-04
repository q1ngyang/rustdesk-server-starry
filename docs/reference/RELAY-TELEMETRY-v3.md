# Relay Telemetry v3

**English** | [简体中文](RELAY-TELEMETRY-v3.zh-CN.md)

Relay Telemetry v3 is the authenticated HBBR snapshot for patch-v1.3.2. It
retains the v2 HMAC/mTLS trust boundary, freshness, instance/sequence rules,
load fields, public-probe privacy, and AKR1 counters. Its canonical schema and
fixture are
[`telemetry.schema.json`](../../contracts/relay-telemetry/v3/telemetry.schema.json)
and
[`telemetry.example.json`](../../contracts/relay-telemetry/v3/telemetry.example.json).

The required `fast_media` object adds typed renewal capability, the 2048-packet
replay window and maximum accepted forward jump, bounded transition/recovery
and absolute-session limits, configured per-IP/global byte budgets, aggregate
reservations, renewal outcomes, post-expiry rebinds, replay rejection classes,
admission outcomes, minimum remaining grant TTL, and the number approaching
expiry. `reserved_bytes_per_second` never exceeds the configured global budget;
`peak_per_ip_reserved_bytes_per_second` never exceeds its per-IP budget.

HBBS accepts schema 3 only after authenticating the response and enforcing the
existing sequence, instance restart, timestamp, and freshness rules. Renewal
eligibility additionally requires `fast_media.protocol = 1`,
`fast_media.renewal_protocol = 1`, enabled/healthy state, and exact configured
UDP port. Schema v2 remains valid for bootstrap-only FastMedia and schema v1
for ordinary Relay quality/load behavior.

All additions are fixed aggregate dimensions. The payload contains no client
IP, session UUID, allocation/request ID, nonce, token, grant, or media bytes.
HBBR still never connects to Control API or Kessoku; HBBS pulls and validates
the snapshot, and the Control Agent exposes only its sanitized copy.

