# patch-v1.1.0 new features

patch-v1.1.0 adds an opt-in WebSocket Signal path for RustDesk clients in
restricted enterprise networks while preserving native signalling and P2P as
the normal, faster default.

## Added

- Persistent `/ws/id` registration with the same RegisterPk identity checks as
  native registration.
- A single-writer WebSocket transport with a bounded outbound queue,
  generation-safe route replacement, heartbeat, idle timeout, global and
  per-effective-IP session limits, and registration rate limiting.
- Cross-transport signalling for WSS-to-WSS and WSS-to-native sessions. If
  either endpoint uses WSS, HBBS explicitly forces Relay and never advertises a
  P2P path that the restricted endpoint cannot use.
- Transport-aware Relay selection: native-only sessions retain the existing
  online list, WSS sessions require a healthy `/ws/relay` endpoint, and mixed
  sessions require both native and WSS health on the same Relay node.
- Certificate- and hostname-verified `wss://.../ws/relay` probes. No TLS bypass,
  ping substitute, or HTTPS-200 substitute is accepted.
- Strict schema version 2 with trusted-proxy CIDRs, exact Origin allow-listing,
  WSS endpoint coverage, queue/session/rate limits, and timing validation.
- Loopback management commands `websocket-status` (`ws`) and
  `test-geo <IP_A> <IP_B> [native|wss|mixed]`.
- An automated official-HBBR mixed WebSocket/native bidirectional payload test.
- A real-HBBS process integration test using a runtime-generated test CA and a
  hostname-valid certificate. It covers `/ws/relay` TLS Upgrade,
  RegisterPeer/RegisterPk, keep-alive, binary heartbeat, WSS-to-WSS and both
  mixed signalling directions under `RUST_LOG=warn`.

## Compatibility and operational model

- Existing `version: 1` configuration remains valid and WebSocket Signal stays
  disabled. A v1 document containing `websocket_signal` is rejected explicitly.
- A client with WebSocket disabled follows the patch-v1.0.0 native behaviour,
  including P2P preference and Secure TCP where configured.
- WebSocket is a per-client escape hatch for constrained networks. It is not
  automatically enabled and does not replace native P2P.
- WSS data sessions are Relay-only. The initial implementation keeps the
  official HBBR and verifies its mixed transport contract in CI.

## Upgrade note

To enable the feature, copy the existing v1 configuration to a backup, create a
`version: 2` configuration, configure exactly one certificate-valid
`/ws/relay` endpoint for every `relay_servers` entry, then enable
`websocket_signal.enabled`. Keep port 21118 reachable only through the trusted
reverse proxy in production.

## Repository validation status

The overlay has been replayed repeatedly on a clean official 1.1.16 checkout,
with an unchanged second application and a clean `git diff --check`. The locked
Linux build passed 30 library tests, all binary checks, the real-HBBS WebSocket
integration test, and the official-HBBR mixed Relay test in both registration
orders with bidirectional payloads. Compose and rustfmt checks also pass.

The official HBBR reserves non-WebSocket loopback connections for its local
management channel, so the native side of the mixed test uses a non-loopback
local address. The test also models realistic ordered registration with a
100 ms gap; a synthetic zero-gap pair can trigger the upstream HBBR `PEERS`
remove/insert race and is not treated as a Starry signalling contract.

Full TLS reverse-proxy coverage for `/ws/id`, Relay-failure failover, the
1,000-session/30-minute stress gate, real-client desktop control, seven-Relay
production ingress, and rollback remain deployment acceptance work. Release
artifacts, SBOM, and the multi-architecture manifest are independently gated by
the corresponding GitHub Actions release run. Neither result is evidence of a
production rollout.

# patch-v1.1.0 版本新特性

patch-v1.1.0 为受限企业网络中的客户端增加按需 WebSocket 信令能力，同时保留
原生信令与 P2P 作为日常默认高速路径。

- 客户端关闭 WebSocket 时，继续按原生/P2P 路径工作。
- 客户端显式开启 WebSocket 时，支持持久 `/ws/id` 注册与心跳。
- 任一端使用 WSS 时强制走 Relay，支持 WSS↔WSS 与 WSS↔原生混合会话。
- Geo Relay 分配会按 native、wss、mixed 三种传输要求过滤节点。
- `/ws/relay` 健康检查验证 DNS、TCP、TLS 证书/域名和 WebSocket 101 Upgrade。
- schema v2 增加可信代理、精确 Origin、帧/队列/session/限速和 Relay endpoint
  完整覆盖校验；原有 v1 配置仍可读取且 WSS 默认关闭。
- 新增 `websocket-status` 与传输感知 `test-geo` 管理命令，并在 CI 中锁定
  官方 HBBR 的 WSS/原生混合双向数据契约。
- 新增真实 HBBS 进程集成测试：运行时生成测试 CA 与域名匹配证书，在
  `RUST_LOG=warn` 下覆盖 `/ws/relay` TLS Upgrade、RegisterPeer/RegisterPk、
  keep_alive、空二进制心跳、WSS↔WSS 和两个方向的混合信令。

## 仓库验收状态

overlay 已在官方 1.1.16 干净源码上重复重放，第二次应用无文件变化，且
`git diff --check` 通过。锁定依赖的 Linux 构建已通过 30 个库单测、全部二进制
检查、真实 HBBS WebSocket 集成测试，以及两种登记顺序下的官方 HBBR
原生/WSS 混合双向载荷测试；Compose 与 rustfmt 检查也已通过。

官方 HBBR 会把非 WebSocket loopback 连接留作本机管理通道，因此混合测试的
原生端使用非 loopback 本机地址，并以 100 ms 间隔模拟真实登记顺序；零间隔的
合成登记可能触发上游 HBBR `PEERS` 删除/插入竞态，不作为 Starry 信令契约。

完整 TLS 反向代理 `/ws/id`、Relay 故障切换、1,000 session/30 分钟压力门槛、
真实客户端桌面控制、七 Relay 生产入口和回滚仍属于部署验收。发布物、SBOM 与
多架构 manifest 由对应 GitHub Actions 正式发布运行独立门禁；两者都不能作为
生产环境已经上线的证据。
