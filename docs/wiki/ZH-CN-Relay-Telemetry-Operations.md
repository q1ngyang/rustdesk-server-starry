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

所有 Control 维度都有界：offer/fallback reason 是固定字段；每 Relay selection 最多 256 个配置 Relay key，另有 overflow 计数。API 不返回客户端 IP、session/allocation 标识、nonce 或原始 report。

## 滚动升级与回滚

升级顺序：

1. 先部署 TLS/mTLS 策略和 secret file，不改变流量。
2. 滚动 HBBR patch-v1.3.0；已有数据会话与官方客户端保持兼容。验证公开 probe 不含 load、受认证遥测为 schema 1。
3. 滚动 HBBS patch-v1.3.0；先关闭质量功能，或把相关 Relay 明确列为 legacy fallback。验证 fresh、实例/sequence 稳定以及 Control inventory。
4. 逐步启用质量候选，再删除临时 legacy 声明。
5. 最后滚动 Control Agent；它只读取 HBBS local control，不连接 HBBR。

回滚时先关闭 `relay_quality`，把 HBBS endpoint 恢复为 `/ws/relay` 并删除 `telemetry_secret_file`，再回滚 HBBS。随后可独立回滚 HBBR；普通 `relay_server`、native TCP、WS/WSS 配对和官方客户端始终可用。不要让旧 HBBS 读取含新遥测字段的 schema-v4 配置。
