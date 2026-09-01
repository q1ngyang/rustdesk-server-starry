# Relay Quality 协议 v1

[English](RELAY-QUALITY-PROTOCOL-v1.md) | **简体中文**

状态：2026-09-01 **FROZEN**。Akari 只能依据
[`contracts/relay-quality/v1/FROZEN`](../../contracts/relay-quality/v1/FROZEN)
记录 SHA-256 的 canonical protobuf 开始 wire 实现。冻结时尚未发布任何
patch-v1.3.0/Relay Quality 构建，因此可在发布前原位修订 v1；`strategy: eager` 明确保留
先前一次探测全部候选的行为，无需新增 v2。

本协议是 RustDesk `RendezvousMessage` 的 Starry/Akari 私有增量扩展，不改号、不改变
任何官方字段类型。官方客户端忽略未知高 tag，继续只使用 HBBS 普通 `relay_server`。

## 能力协商与字段绑定

Akari 发起端以 `PunchHoleRequest.relay_quality_protocol = 1` opt-in。HBBS 只接受精确 v1，
并返回 `RelayQualityOffer.protocol_version = 1`；零值或未知版本完整走 legacy 路径。只有
HBBS 受认证遥测明确给出 `relay_probe_protocol >= 1` 与
`relay_load_protocol >= 1` 的 HBBR 才能成为质量候选，绝不从版本字符串推断能力。

| 容器 | 增量字段/tag |
| --- | --- |
| `PunchHoleRequest` | `relay_quality_protocol = 100` |
| `PunchHole`、`FetchLocalAddr` | `relay_quality_offer = 100` |
| `PunchHoleSent`、`LocalAddr` | `relay_quality_report = 100` |
| `PunchHoleResponse` | offer `100`、peer report `101`、decision `102` |
| `RequestRelay` | controller report `100`、decision `101`、allocation ID `102` |
| `RelayResponse` | peer report `100`、decision `101` |
| `RendezvousMessage.union` | HBBR probe request `100`、response `101`；阶段 report `102`、offer `103`、decision `104`、cancel `105` |

canonical 定义见
[`rendezvous-extension.proto`](../../contracts/relay-quality/v1/rendezvous-extension.proto)。

稳定数值如下：

| 类型 | 数值 |
| --- | --- |
| strategy | `1 adaptive`、`2 eager` |
| stage | `1 primary`、`2 expanded`、`3 eager` |
| endpoint role | `1 controller`、`2 target` |
| decision reason | `1 primary_accepted`、`2 expanded_best_score`、`3 partial`、`4 hysteresis`、`5 legacy_fallback`、`6 probe_failure`、`7 manual_override` |
| cancel reason | `1 p2p_succeeded`、`2 client_abort` |

`RelayQualityDecision.reason` tag 5 是冻结前占位字段。发送端必须留空，客户端只解释
`reason_code`，不得把服务端自由文本直接显示给用户。

## GEO primary 自适应流程

GEO 规则先产生有序 eligible Relay 列表。普通 GEO 结果是 `fallback_relay`；只有它同时是
第一个新鲜、能力兼容的质量候选时才创建质量 allocation。GEO primary 若为 legacy、
stale、不健康或不可探测，HBBS 不会暗中换另一个 primary，而是不发 offer 并完整使用传统
GEO/failover。legacy HBBR 不占用 `max_candidates`。

默认 `adaptive` 流程完全由服务端协调：

1. HBBS 创建 16 字节 allocation ID、全新 16 字节 primary stage token 与总 deadline。
   target 的 stage-1 offer 只包含 GEO primary；原生端使用 `tcp://`，WSS 端使用配置的
   `wss://.../ws/relay`。
2. target 可在 `PunchHoleSent` 或 `LocalAddr` 返回绑定的 primary report；
   `PunchHoleResponse` 向 controller 下发 stage-1 offer，并在存在时附带净化后的 target
   report。
3. controller 在 `RequestRelay` 中上报 primary report 并回显 allocation ID。只有 HBBS
   使用所有可用端点报告与可信 load 解释 `primary_accept_score` 和
   `primary_max_loss_basis_points`，客户端不复制评分策略。
4. primary 达标时立即选择。HBBS 把同一字节值写入 `RequestRelay.relay_server` 与
   `RelayQualityDecision.relay_server` 后才转发请求。
5. 未达标时 HBBS 截留请求，生成全新 stage-2 token，同时向两端下发只含其余候选的
   top-level expansion offer。同阶段不同候选并发探测，同候选样本仍按协议顺序执行。
6. top-level report 被绑定和去重。HBBS 综合 stage-1 primary、stage-2 其余候选以及仅由
   自己认证取得的 HBBR load，应用 hysteresis 后向双方发送完全相同的最终 decision。
   controller 携带 allocation ID 重试 `RequestRelay`；HBBS 覆盖普通字段和扩展字段后才
   转发，因此客户端无法自行选择或篡改最终 HBBR。

`strategy: eager` 使用一个 stage-3 offer 立即下发全部质量候选，保留冻结前 v1 的
eager-all-candidates 流程，并复用相同 allocation、绑定、评分、decision 与隐私规则。

## 状态、重放与 deadline

每份 report 同时绑定协议版本、allocation ID、当前 stage、当前 stage token、端点角色、
活动配置 generation、精确信令 route、发起端 IP、目标端 IP 与已下发候选集合。每个阶段
必须为每个候选恰好提交一项结果，且 `attempted ==` 该阶段 `probe_samples`。完全相同的
重复包幂等；冲突重复、旧 stage/token、角色颠倒、未知候选及其他 route/IP 的重放均拒绝。
最终阶段的精确重复只重发既有最终 decision，绝不重新选择。

单次尝试硬超时为 `probe_timeout_ms`；`stage_deadline_unix_ms` 是阶段上限，
`total_deadline_unix_ms` 是 allocation 上限。HBBS 以服务端单调时钟判定，客户端墙钟只供
参考。HBBS 每 100 ms 的有界 tick 最多完成 64 个已到期且请求 Relay 的 allocation。
`allocation_ttl_seconds` 仅作更晚的崩溃清理，不延长 report deadline。

adaptive 配置校验定义：

```text
window(samples) = samples * probe_timeout_ms
                + (samples - 1) * probe_interval_ms

required = 2 * (p2p_probe_grace_ms + window(primary_probe_samples))
         + window(probe_samples)
         + 1000 ms 信令余量
```

`report_timeout_ms` 必须不小于 `required`；eager 必须容纳两个完整 probe window。天然
不可完成的配置在激活前拒绝。候选最多 5 个，allocation/decision/cache map 受
`max_allocations` 硬限制，deadline 每 tick 分发也有上述硬上限。

若两端均声明 v1，HBBS 在阶段 deadline 前等待两端可用 report。被控端是官方/legacy
客户端时，合法的 controller-only report 会明确使用 missing-report penalty，reason 为
`partial`；primary 不佳则执行 controller-only expansion。controller 测量缺失或失败时不能
选择质量候选，按 `probe_failure` 使用普通 fallback。

任一端 P2P 成功后发送绑定的 `RelayQualityCancel(reason=p2p_succeeded)`；HBBS 立即删除
allocation，best-effort 通知另一端，不等待探测 deadline。不能取消的 legacy 对端仍由 TTL
兜底清理。

Force Relay/auto-relay、对称 NAT、WSS 和 mixed 均不能绕过协调：v1 allocation 必须先取得
最终 decision 才能连接 HBBR。完全不带 v1 allocation 的直接 `RequestRelay` 保持官方
legacy 行为。

## 探测指标、评分与 HBBR 边界

HBBR 一次性 `RelayProbeRequest/Response` v1 wire 不变，每次探测使用全新 16 字节 nonce。
RTT 是成功往返的四舍五入平均值（最小 1 ms）；jitter 是相邻成功样本差值绝对值的平均；
loss 由 attempted/succeeded 推导。零成功时 RTT 与 jitter 必须均为零。

HBBS 用配置的 RTT、jitter、loss、load 权重生成 `0..10000` 分；双端取较差 loss/jitter，
RTT 偏重较差端，缺失报告使用配置惩罚。候选必须从 controller 可达；存在 target report
时也必须从 target 可达。cache key 只保留对称 IPv4 `/24` 或 IPv6 `/56` 前缀。

公开 offer 和 `RelayProbeResponse` 不携带详细 load。HBBS 只使用新鲜且受认证的
`/ws/telemetry`。官方 HBBR 可以保留为普通 `relay_server` fallback，但绝不主动探测。
HBBR 现有 per-IP/global probe 限流及 native/WS/WSS 字节转发路径不变。

## 兼容与可观测性

| 发起端 | 目标端 | 结果 |
| --- | --- | --- |
| 官方客户端 | 任意 | 不 opt-in，GEO/官方 `relay_server` 行为不变。 |
| Akari v1 | 官方客户端 | controller-only 自适应评分；明确 `partial`，探测失败则 legacy fallback。 |
| Akari v1 | Akari v1 | 双方按阶段同步，收到同一个服务端最终决定。 |
| Akari v1 且 `strategy=eager` | Akari v1 | 按 v1 eager 语义立即探测全部候选。 |
| 任意客户端 | 官方 HBBR | 只作普通 fallback，绝不成为质量候选。 |

Control API 只暴露有界聚合：当前 protocol/strategy、primary probe/accept、expansion、P2P
cancel、估算节省尝试数、expanded decision、timeout、report outcome、fallback reason、
hysteresis/cache hit 及有上限的 per-Relay 选择计数。不得返回单次 report、完整客户端地址、
session UUID、allocation ID、stage token、nonce 或连接 token。Kessoku 只观察 HBBS；
Kessoku 与 Akari 都不直接连接 HBBR telemetry。
