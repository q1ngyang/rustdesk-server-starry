# Relay Telemetry v3

[English](RELAY-TELEMETRY-v3.md) | **简体中文**

Relay Telemetry v3 是 patch-v1.3.2 的认证 HBBR snapshot。它完整保留 v2 的
HMAC/mTLS 信任边界、新鲜度、instance/sequence、负载字段、公开 probe 隐私和
AKR1 计数。规范 schema 与 fixture 位于
[`telemetry.schema.json`](../../contracts/relay-telemetry/v3/telemetry.schema.json)和
[`telemetry.example.json`](../../contracts/relay-telemetry/v3/telemetry.example.json)。

必填 `fast_media` 对象新增 typed 续期能力、2048 包 replay window 与最大前向
sequence 跳变、有界过渡/恢复/绝对会话期限、配置的 per-IP/global byte budget、
聚合预留、续期结果、到期后 rebind、replay 拒绝分类、准入结果、最小剩余授权
TTL 及即将到期数量。`reserved_bytes_per_second` 不得超过 global budget，
`peak_per_ip_reserved_bytes_per_second` 不得超过 per-IP budget。

HBBS 必须先认证响应，并继续执行 sequence、实例重启、时间戳和新鲜度规则，才会
接受 schema 3。续期资格还要求 `fast_media.protocol = 1`、
`fast_media.renewal_protocol = 1`、enabled/healthy，以及与配置完全一致的 UDP
端口。schema v2 仍可用于只能 bootstrap 的 FastMedia，schema v1 仍可用于普通
Relay quality/load。

所有新增项都是固定维度聚合。payload 不含客户端 IP、session UUID、allocation/
request ID、nonce、token、grant 或媒体字节。HBBR 仍不连接 Control API 或
Kessoku；由 HBBS 主动拉取和校验，Control Agent 只暴露脱敏副本。

