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

telemetry schema 2 中，已启用 FastMedia listener 连续超过两个 interval 不健康、
`listener_failures` 持续增长，或在有实际 active 负载时 `rate_limited`、
`replay_rejected`、`grant_rejected` 持续增长，适合告警。HBBS 声明 UDP port 与新鲜
telemetry 端口不一致也应告警。`hello_accepted`、单次 cookie/bind 失败、rebind、累计
forwarded packet/byte、active allocation/stream 和偶发 drop 主要用于诊断/容量规划，除非
同时出现 fallback 或用户故障。AP 迁移测试中 rebind 上升是预期行为；listener 重启不
代表可靠 HBBR 会话失败。

所有 Control 维度都有界：offer/fallback reason 是固定字段；每 Relay selection 最多 256 个配置 Relay key，另有 overflow 计数。API 不返回客户端 IP、session/allocation 标识、nonce 或原始 report。

## 滚动升级与回滚

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
