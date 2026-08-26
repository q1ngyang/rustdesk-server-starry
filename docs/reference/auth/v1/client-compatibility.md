# RustDesk 1.4.9 controller-only compatibility trace

**English** | [简体中文](client-compatibility.zh-CN.md)

Pinned client source: `rustdesk/rustdesk@1.4.9`.

| Flow | Client source behavior | Starry v1 profile |
| --- | --- | --- |
| `PunchHoleRequest` denial | `src/client.rs` checks non-empty `PunchHoleResponse.other_failure` before mapping the existing failure enum and returns that text as the connection error. | Return `failure=OFFLINE` plus `other_failure="connection authorization failed"`. |
| Direct `RequestRelay` denial | `src/client.rs` checks `RelayResponse.refuse_reason` and returns it before creating the HBBR connection. | Return `refuse_reason="connection authorization failed"`. |
| Controlled endpoint registration | `src/rendezvous_mediator.rs` registers the endpoint independently, receives forwarded `PunchHole`/`RequestRelay`, and does not need a controller user JWT. | Do not require controlled-side login; authorize only the initiating `PunchHoleRequest` or direct `RequestRelay`. |
| UDP `PunchHoleRequest` | Starry's pinned 1.1.16 overlay does not dispatch UDP `PunchHoleRequest`. Supported initiation reaches HBBS over native TCP, Secure TCP, or `/ws/id`. | Keep UDP explicitly unsupported; it must not query a target, record a punch request, or allocate a Relay. |

The existing protobuf fields are sufficient. This contract therefore does not
modify `rendezvous.proto` or introduce a new failure enum. Client-facing text
is stable and deliberately does not reveal target existence or internal token
failure reasons.

See the [authentication profile](profile.md) for the verification rules.
