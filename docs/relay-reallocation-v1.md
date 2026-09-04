# Relay Reallocation v1

Contract status: FROZEN. Runtime release status: IMPLEMENTATION_PENDING. Akari and Kessoku may implement against the immutable contract, but no runtime may be described as publishable until the integration gates pass.

Relay Reallocation v1 is independent from the frozen Relay Quality v1 protocol. It advertises the typed `relay_reallocation=1` capability and uses additive Rendezvous oneof tags 108–116. RequestRelay tag 103 and RelayResponse tag 102 advertise per-endpoint support. Zero or absent means unsupported and preserves the current session.

The authenticated requester supplies only the current binding, a 16-byte idempotency ID and optionally a configured node ID. It cannot supply a Relay hostname or probe URL. HBBS binds the request to the current session UUID, controller/target role and route, current Relay, session generation, configuration generation and deadline. Simultaneous conflicts sort by earlier deadline, then controller role, then lexicographic request ID.

HBBS derives eligible targets from the current health snapshot. A target must have explicit node metadata, fresh health and `relay_probe_protocol=1`. Official HBBR remains an ordinary fallback only. Display names, regions, relay identities, probe URLs and aliases are exact configuration; no domain inference, redirects or client-selected hosts are allowed.

The state machine is one bounded manual evaluation followed by `prepare -> ready(controller + target) -> commit -> commit_ack(controller + target) -> drain old`. Both ready messages must carry the same new reliable-path binding digest. HBBS serializes one commit payload, including exactly one ordinary `relay_server`, and sends the same bytes to both endpoints. Until both commit ACKs arrive, the old reliable session remains authoritative. Reject, timeout, configuration change, route loss, digest disagreement or connection failure rolls both sides back.

Prepare fences the old FastMedia renewal chain. HBBS signs new controller and target grants for the selected Relay; unsupported UDP keeps FastCompat or standard reliable Relay. New allocation/session/key/replay domains must be used. Late renewal or old grants cannot install after the generation changes. AKR1 kinds 1–5 and Relay Quality v1 remain unchanged.

Control API surfaces only typed configuration and bounded counters. It never exposes a session UUID, request/reallocation/allocation ID, full address, probe URL, report, nonce, token, grant, capacity, bandwidth or key. Kessoku is not in the arbitration, signing, probe or media path.

Runtime release gates: dual-client on-demand probe exchange and scoring; dual patched-HBBR native/WSS/mixed switchover; old-path drain and rollback under three failure points; FastMedia renewal race and old-grant replay; controlled-clock ten-minute session; high-concurrency bounds; real NAT/UDP-blackhole soak. These gates do not block contract freezing, but they keep Akari, Kessoku and runtime integration status BLOCKED and prohibit preview/stable/latest publication claims.
