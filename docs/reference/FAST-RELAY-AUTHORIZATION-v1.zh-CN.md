# 极速 Relay 授权协议 v1

[English](FAST-RELAY-AUTHORIZATION-v1.md) | **简体中文**

极速 Relay 授权协议 v1 是 Starry/Akari 的增量扩展。只有连接鉴权和 Relay
质量选择均成功后，HBBS 才会授权 Akari 使用 `FastCompat`。patch-v1.3.0
首个版本的媒体数据仍经过现有可靠 HBBR 数据流，绝不授权或广播
`FastMediaV1` Relay UDP。

官方 RustDesk 客户端继续兼容：扩展只使用官方协议未知的高位 protobuf
tag，不修改任何官方字段或枚举；扩展不存在或被拒绝时，普通
`relay_server` 流程保持不变。

## 线协议绑定

叠加构建会向上游 `rendezvous.proto` 注入以下增量字段：

| 官方消息 | 新增字段 |
| --- | --- |
| `RequestRelay` | `bytes fast_relay_authorization = 64` |
| `RelayResponse` | `bytes fast_relay_authorization = 64` |

字段内容是 libsodium Ed25519 combined signed message：

```text
signed_authorization = signature[64] || protobuf(FastRelayAuthorization)
```

HBBS 使用现有 Ed25519 私钥签名。Akari 使用正常 RustDesk 运行时已经获得的
HBBS 公钥验签、打开 combined message，然后解析以下载荷：

```protobuf
message FastRelayAuthorization {
  uint32 version = 1;
  string session_uuid = 2;
  uint64 expires_at = 3;
  bool allow_fast_compat = 4;
  bool allow_fast_media_v1 = 5;
  uint32 max_bitrate_kbps = 6;
}
```

机器可读定义位于
[`contracts/fast-relay/v1/rendezvous-extension.proto`](../../contracts/fast-relay/v1/rendezvous-extension.proto)。

## 签发顺序与不变量

HBBS 仅在以下检查依次成功后签发：

1. 连接鉴权返回严格的 `allow`。在 `audit` 模式下，本应拒绝的请求也不会
   获得授权。
2. 信令使用 Secure TCP 或已配置的 WebSocket 信令路径。若 WSS 在反向代理
   终止 TLS，部署必须阻止公网直接访问 HBBS 的明文 WebSocket 监听端口。
3. Relay 质量协议 v1 已针对发起端 IP、目标端 IP、会话 UUID、allocation
   ID 和当前配置 generation 生成最终且有来源绑定的选择。
4. 当前策略有效：授权 TTL 为 `30..300` 秒，码率上限为
   `1000..200000` Kbit/s。
5. HBBS 签名密钥和系统时钟可用，且来源签名限流允许生成新签名。

签名载荷固定为 `version = 1`、`allow_fast_compat = true` 和
`allow_fast_media_v1 = false`。服务端先把最终 Relay 写入官方
`relay_server` 字段，随后才生成授权；目标端 `RequestRelay` 与控制端
`RelayResponse` 收到完全相同的签名字节。

任一步失败都会把 tag 64 置空并增加有界运行计数，不会拒绝或延迟标准
RustDesk Relay 流程，也不会改变已经完成的 Relay 质量决策。

## 重放、重试和隐私控制

- 缓存键把会话 UUID 与规范化后的发起端、目标端 IP 绑定；记录有效期间，
  不允许其他端点组合复用 UUID。
- 响应查找还绑定响应目标端 IP 和最终 Relay 标识。
- 合法重试复用原 Relay 质量决策和完全相同的签名字节，不延长有效期，
  也不额外消耗签名额度。
- 记录受签名过期时间与 Relay 质量 allocation TTL 的较早实际边界约束，
  映射总量受 `relay_quality.max_allocations` 限制。
- 每个规范化来源 IP 每分钟最多生成 120 个新签名。
- 日志不包含会话 UUID、签名密钥、令牌或签名授权；只可记录选定 Relay
  和截断的匿名 allocation 标签。

若验签或 protobuf 解析失败、协议版本不支持、会话 UUID 不一致、授权已
过期或 `allow_fast_compat` 为 false，Akari 必须拒绝授权。不得因为扩展字段
存在就推断支持 FastMedia；`allow_fast_media_v1` 是权威字段，并且在
patch-v1.3.0 中始终为 false。

## 配置与 Kessoku 契约

schema v4 新增默认关闭的策略：

```yaml
fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
```

启用它必须同时启用 Relay 质量，将连接鉴权设为 `audit` 或 `enforce`，并
启用 Secure TCP 或 WebSocket 信令。Kessoku 通过 Control API capability
`fast_relay_authorization: 1` 发现支持，在标准 schema/plan/apply 流程中管理
`fast_mode`，审计中仅记录策略变化和聚合运行计数。Kessoku 不得接收、保存
或记录 HBBS 私钥、连接令牌、会话 UUID 或签名授权。

`GET /control/v1/relays` 提供签发、复用、送达和 fail-closed 原因的有界
`fast_relay` 计数。这些值是运行遥测，不能证明客户端实际进入了
FastCompat 模式。
