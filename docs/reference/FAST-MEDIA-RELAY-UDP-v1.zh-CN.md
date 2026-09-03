# FastMedia Relay UDP v1

[English](FAST-MEDIA-RELAY-UDP-v1.md) | **简体中文**

本文定义 Starry HBBR 面向 Akari FastMediaV1 的增量 `AKR1` UDP 路由封装。
它不替代普通 HBBR TCP/WebSocket 数据流；控制、输入、音频、剪贴板、文件传输、
协商和回退始终保留在可靠会话。HBBR 永远不取得 Akari 媒体密钥，只转发端到端
加密的 `AKF1` 载荷。

机器可读 wire contract 位于
[`contracts/fast-media/v1/akr1-wire.json`](../../contracts/fast-media/v1/akr1-wire.json)，
wire 契约已由 patch-v1.3.1 契约候选摘要标记为 `FROZEN`。运行时发布仍为
`BLOCKED`，必须通过已记录的 Akari 端到端和网络故障门禁；wire 不可变不代表发布批准。

## 固定头与消息

所有整数均为 little-endian。每个 datagram 以 32 字节固定头开始：

```text
0..4   "AKR1"
4      protocol = 1
5      kind: 1 Hello, 2 Cookie, 3 Bind, 4 Bound, 5 Media
6      role: 1 controller, 2 target
7      reserved = 0
8..16  非零 FastMedia session_id（u64）
16..32 非零 relay_allocation_id[16]
```

| kind | 完整大小 | 字节 32 后的 body |
| --- | ---: | --- |
| Hello (1) | 56 | 随机 nonce `[16]`，再跟八个零字节 |
| Cookie (2) | 56 | 原样 nonce `[16]`、cookie `[8]` |
| Bind (3) | 59..4154 | nonce `[16]`、cookie `[8]`、授权长度 `u16`、1..4096 签名字节 |
| Bound (4) | 32 | 无 |
| Media (5) | 121..授权上限 | 一个完整加密 AKF1 datagram |

HBBR 只接收客户端发来的 Hello、Bind 和 Media；Cookie 与 Bound 是服务端响应。
未知 kind、非零 reserved、全零 ID、截断消息或超过授权上限的消息均被丢弃。

## 无状态 cookie 与防放大

Hello 与 Cookie 大小相同。八字节 cookie 认证来源 IP/端口、端点角色、FastMedia
session ID、Relay allocation ID、nonce 和短周期 epoch。HBBR 只接受当前或上一个
十秒 epoch。cookie secret 只存在于进程内，因此 HBBR 重启会有意使全部 cookie 和
内存绑定失效。

来源地址或端口改变后必须重新取得 cookie。HBBR 不会因 Hello 分配会话状态，并在
计算 cookie 前执行全局及每 IP 的有界 packet/byte admission。

## Bind 校验与状态

Bind 携带[极速 Relay 授权 v1](FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)定义的角色专属
Ed25519 combined authorization。HBBR 先验证 cookie 和签名，再要求以下值与本地监听
和外层固定头完全一致：

- 授权版本 1、尚未过期，且未来过期时间不超过 300 秒；
- `allow_fast_media_v1 = true` 且 `relay_udp_protocol = 1`；
- 精确配置的 Relay identity 和 UDP 端口；
- 精确非零 16 字节 allocation ID 和端点角色；
- `relay_max_datagram` 位于 `608..=1400`，源码率位于
  `1000..=200000` Kbit/s；
- 非空且不超过服务端上限的 session UUID。

第一个有效 Bind 会原子固定 allocation 的一个非零 FastMedia session ID，以及每个
签名角色的一组来源 tuple。冲突 session、角色交换、Relay、allocation 或仍有效 tuple
会被拒绝。角色成功安装后 HBBR 才返回 Bound；controller 和 target 都绑定后才转发媒体。

已经绑定的同一角色在地址/端口变化后，只能使用同一角色授权重新执行新的
Hello/Cookie/Bind。成功 rebind 会立刻撤销旧 tuple，同时保留 replay 与限流状态；每角色
每分钟最多十二次。

## AKF1 校验与转发

HBBR 对每个 Media datagram 只验证 AKF1 明文前导，不解密载荷：

- magic `AKF1` 与协议版本 1；
- 非零 sequence，且位于 128 packet replay window；
- 内层 session ID 与 AKR1 session ID 相同；
- 外层 controller 角色对应 direction 0，target 对应 direction 1；
- 消息来自当前固定 tuple，且大小不超过签名 datagram 上限。

HBBR 精确剥离 32 字节 AKR1 header，把剩余完整 AKF1 字节原样发给另一角色。它不解析
媒体帧、不持有加密密钥，也不终止端到端机密性。

## 资源与流量上限

授权中的编码源上限会转换为 `ceil(max_bitrate_kbps × 1.45)` 的 wire token bucket。
每角色 burst 不超过 `max(256 KiB, 50 ms wire allowance)`。每角色门禁还会叠加有界
每 IP 和全局 packet/byte 额度；持续超限只丢包并计数，绝不关闭可靠 HBBR 会话。

默认半绑定 TTL 为十秒，绑定后 idle TTL 为三十秒，并始终受签名绝对过期时间约束。
allocation 表、来源 IP bucket、每次 tick 清理工作、授权大小和所有导出计数维度都有
硬上限。达到 allocation 或流量上限只拒绝新的 UDP 工作。

## 可靠性与兼容性

FastMedia 是可选第二数据面。UDP 被阻断、授权错误、绑定超时、限流、HBBR UDP
重启、丢包或监听故障，都必须让 FastCompat 或标准可靠媒体路径继续可用。Akari 负责
回退、按有界 backoff 重试、迁移后取得新 cookie，并只在 UDP 路径重新认证后自动重入。

官方 HBBR 和 patch-v1.3.0 HBBR 不广播 `fast_media_relay_udp = 1`，HBBS 不会为其签发
FastMedia 授权。旧 Akari 会忽略或拒绝增量字段，继续使用可靠 Relay。

## HBBR 配置与遥测

已 enrollment Relay 的兼容环境文件可提供以下非 secret 设置：

```text
STARRY_RELAY_PUBLIC_ENDPOINT=relay.example.com:21117
STARRY_RELAY_FAST_MEDIA_UDP_PORT=21119
```

HBBR 还通过 `STARRY_RELAY_FAST_MEDIA_*` 环境变量接收有界 allocation、半绑定/idle
生命周期及每 IP/全局 packet/byte 限制；现有 HBBS 公钥用于验签。FastMedia 能力、监听
健康、active allocation/stream、bind、rebind、转发、丢弃、限流、replay、过期和监听
故障明细，只通过认证 telemetry schema 2 导出。公开 Relay probe 不包含负载或 FastMedia
运行明细。

## 发布门禁

单元测试和真实 HBBR 子进程测试是必要但不充分的证据。只有真实
Akari↔HBBS↔HBBR 集成测试证明双角色授权与绑定、加密转发、UDP 故障后的可靠回退和
自动重入后，该契约才能从候选变为 `FROZEN`。真实设备/AP 漫游、网络整形、码率和长时
soak 仍是独立发布证据。
