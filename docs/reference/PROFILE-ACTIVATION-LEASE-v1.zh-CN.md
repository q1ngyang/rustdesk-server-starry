# Profile Activation Lease v1 协议

[English](PROFILE-ACTIVATION-LEASE-v1.md) | **简体中文**

本文是 Starry patch-v1.3.0 与 Akari Profile 快速切换的 wire 和发布契约。它只扩展
HBBS 注册协议；HBBR 的会话配对、鉴权和数据转发消息均不改变。

## 目标与不变量

Akari 可以在当前 Profile 仍可用时准备另一个 Profile，但只有预期 activation 收到完全
匹配的 Ready ACK 后，才能提交本地切换。旧 socket、延迟 UDP 包、已被替换的 WSS
reader 或旧 Profile 都不能删除最新路由。

下面的完整 tuple 标识单个 HBBS 进程上的一个 activation：

```text
(peer_id, network_identity_uuid, activation_epoch, activation_id,
 route_lease, route_generation)
```

- `activation_id` 是客户端为每次 activation 尝试新生成的 16 个密码学随机字节；
- `activation_epoch` 是该 peer/Profile identity 单调递增的非零值，高 epoch 取代低 epoch；
- `route_lease` 是服务端生成的 32 字节不透明 secret，客户端只原样比较和回传，不派生、
  不记录日志；
- `route_generation` 是服务端进程内非零序列，Native UDP、Native TCP 与 WSS 共用同一
  generation authority；
- route lease 只属于签发它的 HBBS 进程，不是集群 token，也不是 HBBR credential。

## 增量 protobuf 字段

权威注入片段位于
[`contracts/profile-activation/v1/rendezvous-extension.proto`](../../contracts/profile-activation/v1/rendezvous-extension.proto)。
任何官方字段都没有改号、改类型或复用。

| 现有消息 | Tag | 字段 | 方向与含义 |
| --- | ---: | --- | --- |
| `RegisterPk` | 61 | `activation_epoch` (`uint64`) | Akari 注册意图。 |
| `RegisterPk` | 63 | `activation_id` (`bytes`) | 必须正好 16 个随机字节。 |
| `RegisterPkResponse` | 60 | `route_generation` (`uint64`) | HBBS 已提交的 generation。 |
| `RegisterPkResponse` | 61 | `activation_epoch` (`uint64`) | 原样返回已接受意图的 epoch。 |
| `RegisterPkResponse` | 62 | `route_lease` (`bytes`) | 新签发或完全重用的 32 字节 lease。 |
| `RegisterPkResponse` | 63 | `activation_id` (`bytes`) | 原样返回已接受意图的 activation ID。 |
| `RegisterPeer` | 60 | `route_generation` (`uint64`) | 绑定 lease 的心跳/续期。 |
| `RegisterPeer` | 61 | `activation_epoch` (`uint64`) | 绑定 lease 的心跳/续期。 |
| `RegisterPeer` | 62 | `route_lease` (`bytes`) | 绑定 lease 的心跳/续期。 |
| `RegisterPeer` | 63 | `activation_id` (`bytes`) | 绑定 lease 的心跳/续期。 |

Ready ACK 是成功的 `RegisterPkResponse`。只有 `result == OK`、epoch 与 activation ID
和待提交请求逐字节一致、lease 长度为 32、generation 非零时，增强 ACK 才有效。上述
检查全部通过前，Akari **不得**把新 Profile 发布为 active。超时、旧服务端返回的默认
零值、echo 不匹配、lease 畸形或错误 result 都必须保持旧 Profile 已提交状态。

当前相同 epoch 和 activation ID 的重传可以收到同一 lease/generation。确认 transport
清理后的重连或更高 epoch 会取得新 lease/generation。同一 epoch 携带不同 activation ID
或 public key 属于 stale 请求并被拒绝。

## 显式注销

`RendezvousMessage` 新增 oneof tag 62 的 `DeactivatePeer` 和 tag 63 的
`DeactivatePeerResponse`。请求携带完整 tuple：

| `DeactivatePeer` 字段 | Tag | 要求 |
| --- | ---: | --- |
| `id` | 1 | 非空 peer ID。 |
| `network_identity_uuid` | 2 | 正好 16 字节。 |
| `activation_epoch` | 3 | 非零且为当前值。 |
| `activation_id` | 4 | 正好 16 字节且为当前值。 |
| `route_lease` | 5 | 正好 32 字节且属于本节点当前路由。 |
| `route_generation` | 6 | 非零且为当前值。 |

HBBS 只有在原子校验完整 tuple、移除该精确 Native/WSS route 并将 activation 标记为
retired 后才返回 `deactivated: true`。其他情况返回 `false`（畸形请求也可能关闭
transport），但不改变当前路由。响应回显 epoch、activation ID 和 generation，调用方可
关联乱序回复。

WSS 显式注销在发送 ACK 前只 detach 当前 generation。普通 reader 退出和 idle drain
同时比较共享 route generation 与服务端内部 WSS connection ID；该 connection ID 不属于
线上协议，用于确保同一 activation 重试复用 lease/generation 时 `remove_if_current` 仍然
安全。Native route 删除比较同一 generation 和精确 socket 地址。因此 A1→B→A2 完成后
才抵达的 A1 disconnect/deactivation 无法删除 A2。

## 旧客户端兼容

官方客户端不发送扩展字段，继续走现有注册和路由路径；其响应中新增字段保持 protobuf
默认值。一个 ID 在进程内已有增强 lease 时，旧注册不能静默覆盖这条 leased route。

官方或旧 Starry 服务端会忽略增强注册中的未知字段，也不会返回匹配 Ready ACK，因此
合规 Akari 不提交切换。旧服务端同样忽略未知 deactivation oneof arm。这是 fail-closed
功能降级，不要求 fork 官方客户端协议。

## Lease 生命周期与断线清理

心跳只能 touch 精确匹配的当前 route。来自其他 socket 地址的 Native 心跳，或 WSS 正在
持有路由时抵达的 Native 心跳，只会得到 `request_pk: true`，不能迁移路由；传输迁移必须
经过完整且已验证的 `RegisterPk`。显式断线清理会立即清除匹配 route 和 lease，并短期
保留 activation record，使同一 activation 能以新 lease/generation 安全重连。WSS 服务端
drain 采用同一 generation 与 connection 双重安全清理。

45 秒 route lease TTL 仅用于进程崩溃或 disconnect 丢失兜底，不是正常切换机制。过期在
访问时和有界周期维护时执行；空 record 最多保留 15 分钟，用于拒绝 stale epoch 并限制
内存。peer lock、lease、增强 peer ID 和 burst identity registry 各有 100,000 项上限，
容量耗尽时 fail closed。

## 已验证快速重新注册

只有 HBBS 在已有验证记录中找到以下内容完全一致时，才允许快速重新注册：

```text
(peer_id, network_identity_uuid, public_key bytes)
```

public key bytes 只在内存 burst key 中做 SHA-256。该优化不会绕过全局/IP blocker；同一
精确 identity 在滚动 30 秒内最多接受 12 次快速重注册，当前 activation 的完全重试也
计入。未验证或不匹配 identity 继续受普通注册限流约束。完全重试复用当前
lease/generation、不新签发 lease，但仍消耗一个 burst 配额。这是已有 record 的 identity
校验，并不是新增的私钥持有证明协议。

## 多节点行为与 Kessoku 验证

每个 HBBS 都有独立 route table、generation 序列和随机 lease。Akari 应向所有配置的
HBBS 注册 pending activation，按服务实例保存 ACK，并依据 Profile 策略在所需匹配 ACK
齐备后提交。注销时必须把各节点自己的 lease/generation 发回对应签发节点。

Kessoku 可在每个实例调用 `POST /control/v1/peers:verify`，提交 peer ID、UUID、epoch、
activation ID 和 1 至 16 个候选 lease。节点只会为自己当前有效 lease 返回
`registered: true`。lease 与 activation ID 都按 secret 处理，不得出现在 URL、label、
日志或 trace 中。

## 可观测性

`GET /control/v1/capabilities` 声明 `profile_activation_lease: 1`；
`GET /control/v1/relays` 的 `profile_activation` 返回固定限制与以下计数：

| 计数 | 含义 |
| --- | --- |
| `active_leases`、`last_route_generation` | 当前节点状态和 generation watermark。 |
| `leases_issued`、`leases_reused`、`ready_acks` | 成功 lease 生命周期与 ACK 发送尝试。 |
| `fast_reregistrations`、`renewals`、`route_replacements` | 预期的 Profile 切换活动。 |
| `deactivations`、`disconnect_cleanups`、`ttl_expirations` | 正常关闭、transport 清理和崩溃兜底。 |
| `invalid_requests`、`stale_rejections` | 畸形输入或 generation/lease 不匹配。 |
| `rate_limited`、`capacity_rejections` | burst 或有界 registry 的 fail-closed 事件。 |

应告警持续增长的 stale/invalid/rate/capacity rejection、只增长不清理的 lease、Ready ACK
无客户端 commit，以及 TTL 被当作日常切换路径。计数均为聚合值，不暴露 peer ID、UUID、
activation ID、public key 或 lease。

HBBR 没有新增数据消息；继续通过既有 session/load 指标与断线清理观察快速切换压力。

## 发布顺序

1. 先部署 patch-v1.3.0 HBBS/HBBR 与 Control Agent，客户端仍使用旧路径；确认 capability
   1、零值/预期计数、官方客户端注册和 HBBR 转发；
2. 升级 Kessoku，使其理解 capability、按实例 lease 集、`/peers:verify` 和脱敏规则，
   Profile 切换仍关闭；
3. 最后灰度带匹配 ACK 提交规则的 Akari，覆盖 Native UDP/TCP、WSS、A→B→A、乱序包、
   双 HBBS 节点及新旧客户端混用；
4. 仅在 stale/rate/capacity/TTL 计数和 HBBR disconnect/session 压力保持基线内时扩大
   Akari 灰度范围。

## 回滚

先停止 Akari 发起新的 activation 切换并保持最后已提交 Profile；向每个签发 HBBS
best-effort 发送 `DeactivatePeer`，再等待至少 45 秒并加上已配置的 WSS idle/drain 周期。
active lease 归零后，Kessoku 可停止调用新端点。

随后服务端可独立回滚：所有扩展状态只在内存中，不需要数据库迁移。回滚后的服务端忽略
增强请求字段，而 Akari 的匹配 ACK 规则会防止错误提交。不得通过删除或轮换已保存的 peer
identity/public key 来清除 lease。
