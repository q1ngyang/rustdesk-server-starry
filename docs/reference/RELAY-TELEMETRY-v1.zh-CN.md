# Relay Telemetry v1

Relay Telemetry v1 将公开客户端探测与运维负载彻底分离。HBBR 不连接 Kessoku 或 Control API；HBBS 主动拉取、认证、校验并判定新鲜度，Control Agent 只暴露来自 HBBS 的有界聚合。

## 公开探测

`/ws/relay` 继续兼容官方 WebSocket Relay 客户端。Upgrade 响应只可包含 `x-starry-version`、`x-starry-relay-probe-protocol` 和 `x-starry-relay-load-protocol`。`RelayProbeResponse` 只返回请求 nonce、协议版本、能力和可选 Starry 版本，`load` 字段必须缺失。Native TCP 与 WS/WSS 使用同一响应契约。

HBBR 对每个可解析的 `RelayProbeRequest` 在 nonce/版本校验之前实施固定窗口的全局和传输源 IP 限流。默认每分钟全局 10,000、每 IP 120；可通过 `STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE` 与 `STARRY_RELAY_PROBE_PER_IP_PER_MINUTE` 在 1..1,000,000 范围内设置。IP 表最多 4,096 项。仅保留 malformed、unsupported、rate-limited、successful 聚合计数，不记录 nonce 或原始消息。

## 受认证遥测通道

生产环境必须用 TLS 保护 `/ws/telemetry`，优先在内部反向代理启用 mTLS。如果 TLS 在代理终止，Starry 还要求 HBBR 与 HBBS 通过 secret file 共享 HMAC-SHA-512/256 密钥材料：

- HBBR：`STARRY_RELAY_TELEMETRY_SECRET_FILE=/run/secrets/starry-relay-telemetry`
- HBBS endpoint：`telemetry_secret_file: /run/secrets/starry-relay-telemetry`

文件内容为 32..1,024 字节；尾部 CR/LF 会被去除，再以 SHA-256 派生固定 HMAC key。密钥值不得进入 YAML、URL、日志或 Control API。

HBBS 请求头为 Unix 秒时间戳（允许 ±30 秒）、32 位十六进制 nonce，以及对 `starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry` 的小写十六进制 HMAC。HBBR 使用容量 4,096、保留 30 秒的 replay cache。成功时，`x-starry-telemetry` 返回 base64url-no-pad JSON，`x-starry-telemetry-auth` 返回对 `starry-telemetry-response-v1\n{request_nonce}\n{encoded_payload}` 的 HMAC。缺失、畸形、过期、重放或认证失败统一返回不含遥测头的 HTTP 401。

## 指标语义

- `active_sessions` 仅统计两端已配对且取得 admission 的数据会话，转发结束即递减。
- `pending_pairs` 统计未配对首端；每项 generation 防止旧超时删除同 UUID 的新等待项。
- `bandwidth_bps` 是各活动会话 bit/s EMA 之和；单次样本窗口至少 1 秒，α=0.25（`bandwidth_ema_alpha_basis_points=2500`）。
- `capacity_sessions` 是真正执行的 `STARRY_RELAY_MAX_SESSIONS` 上限，不再只是 hint；`capacity_bandwidth_bps` 是 HBBR 总带宽配置。
- `load_basis_points` 取会话利用率和带宽利用率较大值，最大 10,000。
- `STARRY_RELAY_DRAINING` 或 `STARRY_RELAY_DRAINING_FILE` 存在时进入 draining；只拒绝新配对，不中断已有会话。
- `admission_rejections` 聚合容量、draining 和 pending 上限拒绝。

每个 payload 都包含 schema 1、进程实例 UUID、单调 sequence、观测时间、uptime、版本、能力、生命周期 gauge 与聚合计数。同一实例 sequence 或 uptime 不单调时 HBBS 拒绝并 fail closed；实例变化会被接受并计为重启。未来超过 30 秒的观测拒绝；旧观测仍可诊断，但超过 `max_telemetry_age_seconds` 后标记 stale 且不参与质量评分。

官方/legacy HBBR 仍可通过 `/ws/relay` 做普通健康检查，其详细字段安全地返回 `null`，不能成为质量候选，但可继续作为普通 `relay_server` fallback。
