# Relay Quality protocol v1

**English** | [简体中文](RELAY-QUALITY-PROTOCOL-v1.zh-CN.md)

Status: **FROZEN** on 2026-09-01. Akari may implement only the canonical
protobuf whose SHA-256 is recorded in
[`contracts/relay-quality/v1/FROZEN`](../../contracts/relay-quality/v1/FROZEN).
No patch-v1.3.0/Relay Quality build had been published when v1 was frozen, so
the pre-release v1 contract was revised in place. `strategy: eager` preserves
the earlier all-candidate behaviour; no protocol v2 is needed.

This is a private, additive Starry/Akari extension of RustDesk
`RendezvousMessage`. Official fields are not renumbered or retyped. Official
clients ignore the unknown high tags and continue to use HBBS's ordinary
`relay_server`.

## Capability and field bindings

An Akari initiator opts in with `PunchHoleRequest.relay_quality_protocol = 1`.
HBBS accepts exactly version 1 and returns `RelayQualityOffer.protocol_version
= 1`; zero or an unknown version takes the complete legacy path. An HBBR is a
quality candidate only when authenticated HBBS telemetry explicitly reports
`relay_probe_protocol >= 1` and `relay_load_protocol >= 1`. A version string is
never a capability.

| Container | Additive field/tag |
| --- | --- |
| `PunchHoleRequest` | `relay_quality_protocol = 100` |
| `PunchHole`, `FetchLocalAddr` | `relay_quality_offer = 100` |
| `PunchHoleSent`, `LocalAddr` | `relay_quality_report = 100` |
| `PunchHoleResponse` | offer `100`, peer report `101`, decision `102` |
| `RequestRelay` | controller report `100`, decision `101`, allocation ID `102` |
| `RelayResponse` | peer report `100`, decision `101` |
| `RendezvousMessage.union` | HBBR probe request `100`, response `101`; staged report `102`, offer `103`, decision `104`, cancel `105` |

The canonical definitions are in
[`rendezvous-extension.proto`](../../contracts/relay-quality/v1/rendezvous-extension.proto).

Stable numeric values are:

| Kind | Values |
| --- | --- |
| strategy | `1 adaptive`, `2 eager` |
| stage | `1 primary`, `2 expanded`, `3 eager` |
| endpoint role | `1 controller`, `2 target` |
| decision reason | `1 primary_accepted`, `2 expanded_best_score`, `3 partial`, `4 hysteresis`, `5 legacy_fallback`, `6 probe_failure`, `7 manual_override` |
| cancel reason | `1 p2p_succeeded`, `2 client_abort` |

`RelayQualityDecision.reason` at tag 5 is a deprecated pre-freeze placeholder.
Senders leave it empty. Clients use only `reason_code` and must not display a
server-provided free-text reason.

## GEO-primary adaptive flow

GEO rules first produce an ordered eligible Relay list. The ordinary GEO
selection is the `fallback_relay`; it must also be the first fresh,
capability-compatible quality candidate to start a quality allocation. If the
GEO primary is legacy, stale, unhealthy, or otherwise not probeable, HBBS does
not silently substitute another primary: it creates no offer and uses the
traditional GEO/failover result. Legacy HBBR nodes never consume
`max_candidates`.

The default `adaptive` flow is server coordinated:

1. HBBS creates a 16-byte allocation ID, a fresh 16-byte primary-stage token,
   and a total deadline. The target receives a stage-1 offer containing only
   the GEO primary. Endpoint-specific copies use `tcp://` for native and the
   configured `wss://.../ws/relay` URL for WSS.
2. The target may return its bound primary report in `PunchHoleSent` or
   `LocalAddr`. `PunchHoleResponse` gives the controller its stage-1 offer and,
   when present, the target's sanitized report.
3. The controller sends its primary report in `RequestRelay` and echoes the
   allocation ID. HBBS, not either client, evaluates `primary_accept_score`
   and `primary_max_loss_basis_points` using all available endpoint reports
   plus trusted load.
4. If the primary is good enough, HBBS selects it immediately. It writes the
   same bytes to `RequestRelay.relay_server` and
   `RelayQualityDecision.relay_server`, then forwards the request.
5. Otherwise HBBS intercepts the request, creates a fresh stage-2 token, and
   concurrently sends both endpoints a top-level expansion offer containing
   only the remaining candidates. Candidates within the stage are probed
   concurrently; samples for one candidate remain sequential.
6. Top-level stage reports are bound and deduplicated. HBBS combines stage-1
   primary measurements, stage-2 measurements, and only its authenticated
   HBBR load, applies hysteresis, and sends one identical final decision to
   both endpoints. The controller retries `RequestRelay` with the allocation
   ID. HBBS overwrites both ordinary and extension Relay values before
   forwarding, so clients cannot choose or alter the final HBBR.

`strategy: eager` issues one stage-3 offer with every quality candidate and
retains the pre-freeze eager-all-candidates flow. It uses the same allocation,
binding, scoring, decision, and privacy rules.

## State, replay, and deadlines

Every report binds the protocol version, allocation ID, current stage, current
stage token, endpoint role, active configuration generation, exact signalling
route, initiator IP, target IP, and issued candidate set. A report must contain
exactly one result for every candidate in that stage and
`attempted == probe_samples` for that stage. Exact duplicates are idempotent;
conflicting duplicates, old-stage tokens, reversed roles, unknown candidates,
and replay from another route or address are rejected. A finalized-stage
duplicate receives the already final decision and never changes it.

The only route exception is the native target's initial `PunchHoleSent` or
`LocalAddr` response: its source port may differ from the registered target
route when the original controller route, target IP and exact target ID still
match. The report itself must still match allocation, generation, stage/token,
target role and the complete candidate set. WSS, controller and top-level
stage-report routes remain exact, and a conflicting duplicate is rejected.

Each attempt has the offer's `probe_timeout_ms`. `stage_deadline_unix_ms` is
the stage limit and `total_deadline_unix_ms` is the allocation limit. HBBS
checks its monotonic server deadlines; client clocks are advisory only. A
bounded 100 ms HBBS ticker finalizes at most 64 expired requested allocations
per tick. `allocation_ttl_seconds` is later crash cleanup and never extends a
report deadline.

For adaptive configuration validation, define:

```text
window(samples) = samples * probe_timeout_ms
                + (samples - 1) * probe_interval_ms

required = 2 * (p2p_probe_grace_ms + window(primary_probe_samples))
         + window(probe_samples)
         + 1000 ms signalling margin
```

`report_timeout_ms` must be at least `required`. Eager requires two full probe
windows. Impossible configurations are rejected before activation. Candidate
count is at most 5, allocation/state maps are capped by `max_allocations`, and
deadline delivery is bounded as above.

If both endpoints advertised v1, HBBS waits for both usable reports until the
stage deadline. If the controlled endpoint is official/legacy, a valid
controller-only report is explicitly scored with the missing-report penalty
and produces reason `partial`; a poor primary causes a controller-only
expansion. A missing/failed controller measurement cannot select a scored
candidate and uses the ordinary fallback with `probe_failure`.

An endpoint that completes P2P sends a bound `RelayQualityCancel` with
`p2p_succeeded`; HBBS removes the allocation immediately, notifies the other
endpoint best-effort, and does not wait for probe deadlines. Cleanup remains a
TTL fallback for legacy peers that cannot cancel.

Force Relay/auto-relay, symmetric NAT, WSS, and mixed transports do not bypass
coordination: a v1 allocation must have a final decision before either side
connects HBBR. A direct `RequestRelay` without a v1 allocation remains the
official legacy path.

## Probe metrics, scoring, and HBBR boundary

HBBR's one-shot `RelayProbeRequest/Response` v1 wire is unchanged. Each probe
uses a fresh 16-byte nonce. RTT is the rounded mean of successful round trips
(minimum 1 ms), jitter is the rounded mean absolute difference between
consecutive successful samples, and loss is derived from attempted/succeeded.
Zero successes requires zero RTT and jitter.

HBBS scores `0..10000` using configured RTT, jitter, loss, and load weights.
The worse endpoint loss/jitter is used; dual-end RTT weights the worse side.
Missing reports have a configured penalty. A candidate must be reachable from
the controller and, when a target report exists, from the target. Cache keys
retain only symmetric IPv4 `/24` or IPv6 `/56` prefixes.

Public offers and `RelayProbeResponse` do not contain detailed load. HBBS uses
only fresh, authenticated `/ws/telemetry` data. Official HBBR may remain an
ordinary `relay_server` fallback but is never actively probed. Existing HBBR
per-IP/global probe limits and native/WS/WSS byte forwarding are unchanged.

## Compatibility and observability

| Initiator | Target | Result |
| --- | --- | --- |
| Official | Any | No opt-in; unchanged GEO/official `relay_server` behaviour. |
| Akari v1 | Official | Controller-only adaptive scoring; explicit `partial`, or legacy fallback on probe failure. |
| Akari v1 | Akari v1 | Both endpoints stage together and receive one server-selected decision. |
| Akari v1 `strategy=eager` | Akari v1 | All candidates are offered immediately under v1 eager semantics. |
| Any | Official HBBR | Ordinary fallback only; never a quality candidate. |

Control API exposes only bounded aggregates: current protocol/strategy,
primary probes/accepts, expansions, P2P cancellations, estimated attempts
saved, expanded decisions, timeouts, report outcomes, fallback reasons,
hysteresis/cache hits, and capped per-Relay selection counts. It exposes no
individual report, full client address, session UUID, allocation ID, stage
token, nonce, or connection token. Kessoku observes HBBS; neither Kessoku nor
Akari connects directly to HBBR telemetry.
