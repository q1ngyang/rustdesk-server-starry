# Relay 遥测安全与运维

## 部署

每个信任域生成至少 32 字节的随机 secret，以只读方式挂载到 HBBR 与 HBBS 的同一绝对路径。HBBR 设置 `STARRY_RELAY_TELEMETRY_SECRET_FILE`；HBBS 的每个 `relay_health` 条目使用内部 `wss://.../ws/telemetry` URL 和 `telemetry_secret_file`。YAML 只保存路径。不要把密钥值放进环境变量、代理查询参数、访问日志或 Control API 文档。

在防火墙/反向代理上把 `/ws/telemetry` 限制为 HBBS 源网段，并尽可能启用内部 mTLS。代理必须原样转发三个 `x-starry-telemetry-*` 请求头和两个响应头，并禁用缓存。`/ws/relay` 继续公开给官方客户端与 Akari probe；普通 WSS 客户端只能看到能力，不能看到负载明细。

建议的 HBBR 设置：

```text
STARRY_RELAY_TELEMETRY_SECRET_FILE=/run/secrets/starry-relay-telemetry
STARRY_RELAY_MAX_SESSIONS=10000
STARRY_RELAY_PROBE_PER_IP_PER_MINUTE=120
STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE=10000
STARRY_RELAY_DRAINING_FILE=/run/starry/hbbr.draining
STARRY_RELAY_PUBLIC_ENDPOINT=relay.example.com:21117
STARRY_RELAY_FAST_MEDIA_UDP_PORT=21119
STARRY_RELAY_FAST_MEDIA_MAX_SESSION_TTL_SECONDS=43200
STARRY_RELAY_FAST_MEDIA_RENEWAL_TRANSITION_SECONDS=15
STARRY_RELAY_FAST_MEDIA_POST_EXPIRY_RECOVERY_SECONDS=30
STARRY_RELAY_FAST_MEDIA_PER_IP_BYTES_PER_SECOND=33554432
STARRY_RELAY_FAST_MEDIA_GLOBAL_BYTES_PER_SECOND=536870912
```

轮换密钥时，先临时把相关 Relay 作为 legacy fallback，挂载新文件并滚动 HBBR，再滚动或 reload HBBS endpoint。v1 只有一个活动 HMAC key，未协调的原地文件替换可能产生短暂但安全的 fail-closed 遥测空窗。

## 告警与诊断

以下条件持续出现时适合告警，不要因一次采样告警：

- Relay `state != healthy`、`stale=true`，或连续超过两个 health interval 没有受认证遥测；
- 非维护窗口出现 `draining=true`；
- active 达到 capacity 且 `admission_rejections` 持续增长；
- `telemetry_auth_failures`、`reports_invalid`、`reports_binding_mismatch` 或 `reports_late` 上升；
- fallback 比例突增，特别是 `no_reachable_candidate`；
- 以 `primary_probes` 归一化后，`stage_timeouts` 或 `probe_failure` fallback 持续偏高；
- 进程实例频繁变化，或出现 sequence 不单调错误。

以下更适合诊断和容量规划：瞬时 active/pending、带宽 EMA、load basis points、successful/malformed/unsupported probe、每 Relay 选择计数、cache hit 与 hysteresis。只有在同时出现客户端失败或异常来源分布时，才建议把 `probe_rate_limited` 升级为告警。

adaptive 流程计数在没有关联失败时也只用于诊断：
`primary_accepted / primary_probes` 反映 GEO 排序质量，`expansions_triggered`
与 `expanded_decisions` 反映质量层覆盖 primary 的频率，
`estimated_probe_attempts_saved` 衡量分阶段策略节省的探测工作量。
`p2p_cancellations` 通常是健康信号，不应单独触发告警。

telemetry schema 3 中，已启用 FastMedia listener 连续超过两个 interval 不健康、
`listener_failures` 持续增长，或实际负载下 renewal-expired、admission、rate、replay、
grant rejection 持续增长，适合告警。`minimum_remaining_ttl_seconds` 过低、临近到期数量
上升、per-IP/global reservation 接近配置上限也可告警。HBBS 声明 UDP port 与新鲜
telemetry 端口不一致同样应告警。schema 2 仍可证明 bootstrap-only FastMedia，但不能
证明续期能力。

accepted/idempotent renewal、单次 cookie/bind 失败、rebind、角色过渡、replay 拒绝分类、
累计 forwarded packet/byte、active allocation/stream 和偶发 drop 主要用于诊断/容量规划，
除非同时出现可靠回退或用户故障。AP 迁移测试中 rebind 上升是预期行为；listener 重启
不代表可靠 HBBR 会话失败。

所有 Control 维度都有界：offer/fallback reason 是固定字段；每 Relay selection 最多 256 个配置 Relay key，另有 overflow 计数。API 不返回客户端 IP、session/allocation 标识、nonce 或原始 report。

## 滚动升级与回滚

patch-v1.3.2 活动会话续期按以下顺序：

1. 两个 Fast 开关保持关闭，先滚动 HBBR；验证普通 Native/WS/WSS 转发、认证 telemetry
   schema 3、renewal protocol 1、匹配 UDP endpoint、sequence 单调及有界预算。
2. 滚动 HBBS，再滚动 Control Agent；验证 typed capability
   `fast_media_relay_renewal = 1`、新鲜 Relay inventory、有界固定维度且无 secret/session/
   地址字段。Starry 可用 `process_instance_id` 判断重启；Kessoku 必须在入口丢弃，不能
   透传、持久化、索引、写日志或显示。
3. 最后滚动支持续期的 Akari；对有界双端 canary，并要求 renewal 丢失、UDP blocked、
   listener restart、admission failure 和回退/重入全过程可靠桌面流保持连接。

v1.3.2→v1.3.1 时，先停止签发续期，让客户端回退；用 activation ACK 关闭 FastMedia，
并等待 allocation/grant drain。先回滚 HBBS，再回滚 HBBR。schema v5 和持久身份兼容；
旧二进制忽略字段 13–16 与 telemetry-v3 增量。不得删除配对/enrollment 状态。

以下 v1.3.1 顺序仍是历史 v1.3.0 迁移路径。

patch-v1.3.1 升级顺序：

1. 先部署 TLS/mTLS 策略和 secret file，不改变流量。
2. FastMedia 策略保持关闭，先滚动 HBBR patch-v1.3.1。验证可靠会话与官方客户端、
   公开 probe 不含 load、认证 telemetry schema 2、稳定 instance/sequence，以及配置处
   UDP listener 健康。
3. 以 schema v4 或两个 Fast 开关均 false 滚动 HBBS patch-v1.3.1，先验证普通
   Native/WSS/mixed Relay 和新鲜 typed telemetry。
4. 滚动 Control Agent 并验证 schema-v5/OpenAPI fixtures；Agent 不直接抓取 HBBR。
5. 先 canary FastCompat，再在 allowlist Relay/Akari pair 上 canary FastMedia；官方客户端
   和可靠回退测试始终是门禁。

v1.3.1→v1.3.0 回滚先关闭 FastMedia，等待 active authorization、allocation、stream 和
最后 grant expiry 全部 drain。使用 schema-v4 downgrade 预览/导出，并要求 Agent/Relay
证书至少剩余九十天。HBBS 使用兼容 schema v4 回滚，再让 HBBR 使用
`relay-compat.env` 回滚，最后 Control Agent。普通 `relay_server`、native TCP、WS/WSS、
schema-1 telemetry 和官方客户端保持可用；配对/enrollment 状态必须保留，旧二进制不得
改写。
