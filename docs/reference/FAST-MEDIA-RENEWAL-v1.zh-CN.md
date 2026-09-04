# FastMedia 活动会话续期 v1

[English](FAST-MEDIA-RENEWAL-v1.md) | **简体中文**

FastMedia 活动会话续期 v1 是 patch-v1.3.2 在已冻结 Fast Relay 授权 v1 与
AKR1 v1 之上的纯增量 profile。它延长正在运行的 FastMedia allocation，但不改动
Relay Quality v1、普通 `relay_server`、AKR1 头或 kind 1–5。规范 protobuf 位于
[`contracts/fast-media-renewal/v1/rendezvous-extension.proto`](../../contracts/fast-media-renewal/v1/rendezvous-extension.proto)，
精确生命周期和资源规则冻结在
[`renewal-contract.json`](../../contracts/fast-media-renewal/v1/renewal-contract.json)。

只有 HBBS 能使用现有 Starry Ed25519 密钥签名。客户端和 HBBR 均不能自签或延长
授权。该可选控制流失败时，现有可靠 RustDesk/HBBR 连接不得关闭。

## 能力与兼容

HBBR 只在认证 telemetry v3 中用 `fast_media.renewal_protocol = 1` 声明续期。
HBBS 向 Control API 暴露 typed 聚合能力 `fast_media_relay_renewal = 1`，禁止从
版本字符串推断。既有 `fast_media_relay_udp = 1` 保持不变。

- 官方客户端忽略未知 protobuf 字段和消息；
- 旧 Akari 使用 bootstrap 授权，到原期限时安全回退；
- patch-v1.3.1 HBBR 忽略授权字段 13–16，保留原 90/300 秒行为；
- patch-v1.3.2 HBBR 接受旧授权，但把它视为未续期 legacy allocation。

## 新增授权字段

`FastRelayAuthorization.version` 仍为 `1`，字段 1–12 的冻结含义不变：

| Tag | 字段 | 契约 |
| ---: | --- | --- |
| 13 | `fast_media_relay_renewal` | `1` 表示续期 v1；零表示不支持。 |
| 14 | `relay_session_id` | bootstrap 为零；续期后为精确的非零 AKR1 session ID。 |
| 15 | `renewal_sequence` | bootstrap 为零；每次签出双角色授权严格加一。 |
| 16 | `previous_authorization_sha256` | bootstrap 为空；续期后为本角色上一份完整 combined Ed25519 授权的 SHA-256。 |

新授权必须保持相同 session UUID digest、HBBS 选定 Relay、allocation ID、UDP
协议/端口、FastMedia session ID、角色及 datagram 上限。`expires_at` 只能增加，
`max_bitrate_kbps` 只能保持或降低。身份、角色、Relay、协议或上限变化必须 fail
closed，并通过全新 allocation 处理。

## 认证控制消息流

controller 通过 Secure TCP 或 WSS 发送 Rendezvous `oneof` tag 106 的
`FastMediaRenewalRequest`。仅当 connection authentication 精确返回 `allow`，且
规范化 controller IP、session UUID、Relay、allocation、协议、datagram 上限、
当前 bitrate、sequence 及双角色授权哈希全部匹配缓存的 bootstrap 记录时，HBBS
才接受。`requester_role` 必须为 `1`，单独的 allocation ID 永远不构成授权。

WSS 续期继续精确绑定原 controller route。Native Secure TCP 的初始 Relay 响应会
消费一次性连接的 writer，因此续期 v1 只允许在原 controller route IP 和同一认证
controller IP 不变时更换源**端口**；上述其余绑定仍全部精确。源 IP 变化、明文 TCP
或 WSS route 变化均 fail closed。该窄例外不是通用 route rebind，也不会弱化授权链。

首次成功请求固定 controller 选择且 target 已接受的非零 FastMedia session ID；
之后改变它会被拒绝。HBBS 用现有 Starry 密钥分别签出 controller/target 授权，并
在承载已接受请求的同一加密 route 返回 `oneof` tag 107 的
`FastMediaRenewalResponse`。controller 安装本角色授权，并仅通过已认证、端到端
加密的现有可靠桌面会话把 target 授权交给对端。Control API、telemetry 和 Kessoku
均不传递授权。

稳定状态码为：`1 OK`、`2 DISABLED`、`3 UNAUTHENTICATED`、`4 NOT_FOUND`、
`5 BINDING_MISMATCH`、`6 EXPIRED`、`7 TOO_EARLY`、`8 RATE_LIMITED`、
`9 UNAVAILABLE`、`10 INVALID`。客户端不得依赖服务端自由文本显示或判断。

## 幂等、丢包与乱序

请求携带随机 16 字节 `request_id` 和当前双角色 combined grant 的 SHA-256。成功
后 HBBS 只推进一个 sequence，并缓存精确响应。使用相同 request ID 的完全相同
重试在新授权仍有效时返回字节完全相同的授权对，即使旧授权此时已经到期；这属于
已授权签发结果的重放，不是接受过期 grant。旧 sequence 上的不同请求、相同
sequence 的不同授权、跳号或错误上一授权哈希都会被拒绝。

HBBR 分角色保存授权链。两端可短暂相差一个 sequence；仅在两份授权均未过期且
仍处于默认 15 秒的有界过渡窗口内继续 UDP 转发。窗口结束后 UDP fail closed，
但 allocation 会在有界恢复期内保留，可靠连接不受影响。迟到的有效另一角色授权
仍可使双方收敛并恢复 UDP。

新授权继续通过既有 AKR1 Bind（kind 3）和新 Cookie 安装。相同角色的精确重复
幂等；rebind/renewal 均不得清空 AKF1 replay window 或限速累计状态。HBBR 重启
后，仍未过期且由 HBBS 签名的新授权可重建相同 allocation/session；过期授权不行。

## 时间与回退

授权 TTL 仍使用 schema v5 的 `30..=300` 秒配置。客户端开始续期的时间为：

```text
expires_at - min(60, max(30, floor(ttl / 3)))
```

过早请求得到 `TOO_EARLY`。`fallback_before` 固定为到期前十秒。若此时双角色新
授权仍未可用，Akari 保持最后一帧，在旧授权到期前回退 FastCompat；输入、控制和
可靠桌面流不停。重试必须使用有界退避。

可续期且 fully-bound 的活动 allocation 不再因最初 bootstrap 到期或创建满 300 秒
而删除，但仍执行 half-bind TTL、idle TTL、allocation/session 数量、增量清理、
过期角色停止媒体、默认 30 秒到期后恢复期，以及默认 12 小时、范围 10 分钟至
24 小时的绝对会话寿命。不存在永不过期 allocation。

## Replay 与准入

HBBR 的 AKF1 replay window 固定扩展为 2048 包（`32 × u64`），跨 renewal/rebind
保留。重复、窗口外过旧包及大于 1,048,576 的前向跳变均拒绝；跨 64-bit word
移位属于冻结行为。

授权 source cap 转换规则为：

```text
wire_kbps = ceil(source_kbps × 1.45)
wire_bytes_per_second = ceil(wire_kbps × 1000 / 8)
```

HBBR 在首次接受角色 Bind 前，先对 per-role、规范化源 IP 和 global ledger 预留该
额度。同 NAT 的角色/多会话累加；跨 IP rebind 仅在新 IP 有容量时原子迁移；续期
可释放但不能增加额度。HBBS 使用新鲜 telemetry、90% headroom 及有界 Relay/IP
ledger，在签名前降低 cap，并在双授权与 response 中返回实际值。因此默认
32 MiB/s per-IP bucket 不可能签出需要 290 Mbit/s wire cap 的 200 Mbit/s source。
禁止先授权再依赖持续自限速。

## 隐私可观测性

Telemetry v3 和 Control API 只暴露固定维度的成功、幂等、非法、绑定、sequence、
过期、限速、replay 和 admission 聚合；以及活动/预留总量、最小剩余 TTL 与即将
到期数量。不得包含 UUID、客户端 IP、allocation/request ID、nonce、token、stage
token、grant 或媒体内容。Kessoku 只能读取 typed 聚合，不进入签名或媒体路径。
