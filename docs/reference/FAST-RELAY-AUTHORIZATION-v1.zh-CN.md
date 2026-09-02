# 极速 Relay 授权协议 v1

[English](FAST-RELAY-AUTHORIZATION-v1.md) | **简体中文**

极速 Relay 授权协议 v1 是 Starry/Akari 的增量扩展。HBBS 只签发自己选定的
Relay，客户端不能自行选择或替换。官方 RustDesk 客户端会忽略未知的高位
protobuf tag，继续使用普通 `relay_server` 路径。

patch-v1.3.1 保留最初六字段 FastCompat 载荷，并追加 FastMediaV1 Relay UDP
需要的角色绑定字段。授权缺失、损坏、过期、不支持或无法送达，都不能使现有
可靠 HBBR 会话失效。

## 外层信令字段

overlay 在不改动上游字段编号的前提下，为两个官方消息追加同一 opaque 字段：

| 官方消息 | 增量字段 | 接收端 |
| --- | --- | --- |
| `RequestRelay` | `bytes fast_relay_authorization = 64` | 被控端/target |
| `RelayResponse` | `bytes fast_relay_authorization = 64` | 控制端/controller |

字段值是 libsodium Ed25519 combined signed message：

```text
signed_authorization = signature[64] || protobuf(FastRelayAuthorization)
```

HBBS 使用现有 Ed25519 服务端密钥签名；Akari 使用 RustDesk 已经配置的 HBBS
公钥验签。规范增量载荷为：

```protobuf
message FastRelayAuthorization {
  uint32 version = 1;
  string session_uuid = 2;
  uint64 expires_at = 3;
  bool allow_fast_compat = 4;
  bool allow_fast_media_v1 = 5;
  uint32 max_bitrate_kbps = 6;
  uint32 relay_udp_protocol = 7;
  string relay_server = 8;
  uint32 relay_udp_port = 9;
  bytes relay_allocation_id = 10;
  uint32 relay_max_datagram = 11;
  uint32 relay_endpoint_role = 12;
}
```

机器可读定义位于
[`contracts/fast-relay/v1/rendezvous-extension.proto`](../../contracts/fast-relay/v1/rendezvous-extension.proto)。
字段 1–6 保持 patch-v1.3.0 含义不变。旧六字段 FastCompat 授权仍然兼容；新客户端
在 `relay_server` 存在时必须校验它。

## 稳定字段契约

| 字段 | 要求 |
| --- | --- |
| `version` | 固定为 `1`；追加字段不会改变该值。 |
| `session_uuid` | 精确 RustDesk 会话 UUID；HBBS 上限 128 字节。 |
| `expires_at` | Unix 秒；HBBS 策略允许 30–300 秒 TTL。 |
| `allow_fast_compat` | 授权可靠 FastCompat 媒体路径。 |
| `allow_fast_media_v1` | 只有全部 FastMedia 门禁通过时才授权 AKR1。 |
| `max_bitrate_kbps` | 编码源上限，`1000..=200000` Kbit/s。 |
| `relay_udp_protocol` | AKR1 为 `1`；仅兼容授权为零。 |
| `relay_server` | 与普通 `relay_server` 完全相同的 HBBS 最终选择。 |
| `relay_udp_port` | 所选 HBBR UDP 端口；仅 FastMedia 非零。 |
| `relay_allocation_id` | 新鲜、非全零 16 字节值，与公开 UUID 无关。 |
| `relay_max_datagram` | 含 AKR1 的完整 UDP 载荷，`608..=1400`，默认 `1200`。 |
| `relay_endpoint_role` | `1` controller、`2` target；六字段兼容授权为零。 |

FastMedia 会签发两个不同字节串：`RequestRelay` 中 target 授权的角色为 2，
`RelayResponse` 中 controller 授权的角色为 1。两者共享 UUID、过期时间、Relay、
UDP 端口、allocation ID、datagram 上限和码率上限；交换两份授权必须被 HBBR 拒绝。

## 服务端选 Relay 与签发门禁

Relay Quality v1 一旦形成最终 decision 就始终权威；HBBS 把完全相同的 Relay
写入普通 `relay_server` 和签名授权。质量功能关闭、兼容候选不足、超时或使用 legacy
fallback 时，HBBS 也只能签发自己已经通过普通 GEO/failover 流程选出的 Relay，绝不
信任请求携带的 Relay。

HBBS 仅在以下条件全部成立后签发任一授权：

1. 连接鉴权返回精确 `allow`，包括 audit 模式；
2. 信令使用 Secure TCP 或已配置 WebSocket；
3. 普通最终 Relay 已由 HBBS 固定，且存在 Relay Quality decision 时两者完全相同；
4. 策略 TTL、源码率和 datagram 范围有效；
5. 签名密钥、时钟、有界 allocation 缓存和每来源签名额度可用。

FastMedia 还要求所选 HBBR 具有新鲜的认证 telemetry schema 2、明确能力
`fast_media_relay_udp = 1`、健康 UDP endpoint，并且能够向两端送达角色授权。HBBS
不会从 Starry 版本字符串或客户端数据推断能力。门禁失败时仍可在策略允许时签发
FastCompat，并始终保留可靠 Relay。

同一有效 UUID 和规范化端点组合的重试会复用原 Relay 和未过期的两份授权，不延长
过期时间，也不再次消耗签名额度。UUID 冲突、端点组合冲突、最终 Relay 冲突或配置
generation 改变都必须 fail closed。

## 配置

schema v5 让两个模式独立且默认关闭：

```yaml
version: 5
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
    relay_max_datagram: 1200
```

FastMedia 对该授权会话隐含 FastCompat，但开启 `fast_media_v1_enabled` 不会改写配置
中的 FastCompat 开关。Relay 只有在认证 `/ws/telemetry` secret-file 引用旁才能声明
`fast_media_udp_port`；YAML 不保存 secret 值。

## 隐私与可观测性

Control API 只返回有界聚合计数：按角色授权、按模式会话、复用/送达、能力不可用、
可靠回退、无效选择/配置、限流和过期。它不返回 UUID、allocation ID、端点地址、
token、stage token、签名授权或媒体载荷；日志遵循同一规则。

该授权契约是 patch-v1.3.1 的发布候选输入，其 canonical protobuf digest 会写入发布
摘要。AKR1 数据面契约在真实 Akari↔HBBS↔HBBR 可靠回退与自动重入集成门禁通过前
保持 `RELEASE_CANDIDATE_BLOCKED`，不得宣称已经冻结。
