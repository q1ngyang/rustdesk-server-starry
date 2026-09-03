# Relay Telemetry v2

[English](RELAY-TELEMETRY-v2.md) | **简体中文**

Relay Telemetry v2 是 patch-v1.3.1 的认证 HBBR snapshot。它保留 Relay Telemetry
v1 的 transport、request/response HMAC、新鲜度、instance、sequence、load、capacity、
draining、admission 和公开 probe 隐私语义，只新增一个必填、有界 `fast_media` object。
规范 JSON Schema 位于
[`contracts/relay-telemetry/v2/telemetry.schema.json`](../../contracts/relay-telemetry/v2/telemetry.schema.json)。
冻结的非 secret fixture 位于
[`telemetry.example.json`](../../contracts/relay-telemetry/v2/telemetry.example.json)。两者都由
patch-v1.3.1 契约候选摘要绑定，运行时发布仍保持 blocked。

## 信任边界

HBBR 不连接 Kessoku 或 Control API。HBBS 主动拉取证书验证的 `/ws/telemetry`，使用现有
secret-file HMAC（或内部 mTLS 边界）认证，验证 schema/sequence 并判定新鲜度。Control
Agent 只返回 HBBS 的隐私安全聚合视图；客户端提供的 probe/report 永远不是 load 来源。

公开 `/ws/relay` upgrade 与 `RelayProbeResponse` 仍只给出协议 capability 和可选版本，
不包含 active、pending、bandwidth、capacity、draining、admission 或 FastMedia 运行态。

## 从 v1 保留的基础字段

每个 schema-2 snapshot 包含 process instance ID、单调 sequence、Unix 毫秒观测时间、
uptime、精确版本、明确 probe/load protocol、load basis points、已配对 active session、
未配对 pending leg、真实 session capacity、bit/s 带宽 EMA 及 alpha、配置带宽容量、
draining/admission 状态和有界 probe/authentication 计数。

同一 instance 只接受递增 sequence/uptime。instance 改变按重启处理，清除旧 sequence
期望并显式暴露。超过时钟容差的未来 timestamp 验证失败。旧的有效 snapshot 可作为
inventory 证据，但超过 `max_telemetry_age_seconds` 后变 stale，不得用于 Relay Quality
或 FastMedia 授权。

## 必填 `fast_media` object

| 字段 | 含义 |
| --- | --- |
| `protocol` | 精确 AKR1 协议版本 `1`。 |
| `enabled` / `healthy` / `udp_port` | listener 已配置、当前健康与绑定端口；healthy 必然 enabled 且端口非零。 |
| `active_allocations` | 至少一个角色已安装的内存 AKR1 allocation。 |
| `active_streams` | controller 和 target 均绑定的 allocation。 |
| `hello_accepted` / `cookie_rejected` | 无状态 cookie 聚合结果。 |
| `bind_succeeded` / `bind_rejected` / `grant_rejected` | 角色安装与授权结果。 |
| `role_mismatch` / `session_mismatch` / `allocation_mismatch` | 固定、有界拒绝分类。 |
| `rebinds` | 接受的同角色来源 tuple 迁移。 |
| `forwarded_packets` / `forwarded_bytes` | 剥离 AKR1 后转发的 AKF1。 |
| `dropped_packets` / `rate_limited` / `replay_rejected` | 有界数据面拒绝聚合。 |
| `expired_allocations` / `listener_failures` | 生命周期清理与 listener 监督事件。 |

这些值不含 Relay allocation ID、session/connection UUID、nonce、地址、token、grant、
AKF1 字节或媒体内容。Control API 只使用固定字段和已配置 Relay key，不能由不可信客户端
输入创建指标维度。

## 资格与降级

只有 schema 2 新鲜、object enabled/healthy、UDP port 与配置相等且 Relay identity 与
服务端最终选择完全匹配时，HBBS 才把 `fast_media_relay_udp = 1` 视为可用。官方、legacy、
stale、schema-1 或不健康 HBBR 的 FastMedia 状态为 null/unavailable，只能作为普通 Relay
fallback。

Relay 回滚到 patch-v1.3.0 后会通过现有 secret file 恢复 telemetry schema 1；回滚前
HBBS 必须关闭该节点 FastMedia，普通 load telemetry 与 Relay 仍可用。升级顺序为 HBBR、
HBBS、Control Agent。只回滚管理面时先 Control Agent；完整回滚则必须先 drain FastMedia，
再按 HBBS、HBBR 顺序执行。
