# 连接认证

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Connection-Authentication) | **简体中文**

schema v3 可要求 HBBS 在发起新的控制端连接前验证短期用户 JWT。该门禁与账户 API 登录
分离：client 把 connection token 放入既有 RustDesk request 字段，HBBS 在 signalling
边界再次验证。

不能仅因配置语法可加载就启用 `enforce`；必须先完成 issuer 集成和有指标依据的 `audit`
灰度。

## 覆盖请求与传输

同一个 authorization function 覆盖：

| 请求 | 原生 TCP 21116 | Secure TCP 21116 | WSS `/ws/id` | UDP 21116 |
| --- | --- | --- | --- | --- |
| 控制端 `PunchHoleRequest` | 验证 | 验证 | 验证 | unsupported；无响应、无分配 |
| 直接 `RequestRelay` | 验证 | 验证 | 验证 | 不 dispatch |

验证在 frame/protobuf size 检查之后，target lookup、punch-request state、Relay 选择、UUID
创建或 peer 投递之前发生，因此拒绝响应不会暴露 target 是否存在。被控端注册不要求用户
token。

官方 rustdesk-server 1.1.16 的原生被控端 `RegisterPk`/心跳仍使用 UDP；其 TCP listener 对
该注册消息返回 upstream `NOT_SUPPORT`。这不构成认证绕过：发起连接的控制端仍通过原生
TCP/Secure TCP 发送并验证 `PunchHoleRequest`/`RequestRelay`。若被控端所在网络必须完全
禁用 UDP，应改用 WSS 注册；不能在禁用 UDP 后期待原生被控端仅靠 TCP 保持注册。

无需修改 protobuf；精确 client-compatible 拒绝契约由
[客户端兼容性参考](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/reference/auth/v1/client-compatibility.zh-CN.md)
锁定。

## JWT profile

HBBS 只接受：

- 不超过 `max_token_bytes` 的 compact JWT；
- `alg=EdDSA`、精确 `typ=at+jwt`，并带非空显式 `kid`；
- `kid` 唯一对应 `kty=OKP`、`crv=Ed25519`、`use=sig`、`alg=EdDSA` 的公共 JWK；
- 完全匹配的 `iss`、`aud`、`token_use`、完整 `required_scope`、`sub` 与 `user_id`，以及
  在 clock skew 内有效的 `iat`、`nbf`、`exp`。

对称 JWK、私有 `d`、重复 kid、算法 fallback、逐 key 尝试、subject 不一致及畸形时间窗口
都会被拒绝。

## 从 audit 开始

复制 [`config/config.auth-audit.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.auth-audit.yaml)，替换全部示例
hostname/path，以只读方式挂载所需文件，并保持 `mode: audit`：

```yaml
version: 3
connection_auth:
  mode: audit
  issuer: https://kessoku.example
  audience: rustdesk-connect
  jwks:
    file: /var/lib/starry-auth/jwks.json
    url: https://kessoku.example/api/internal/v1/auth/jwks
    refresh_interval_seconds: 300
    max_stale_seconds: 3600
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
  introspection:
    required: true
    url: https://kessoku.example/api/internal/v1/auth/introspect
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
```

容器部署必须把可写持久目录挂载到 `/var/lib/starry-auth`；mTLS CA 与 client identity 继续放在
独立的只读 `/run/secrets/starry-auth` 挂载中。Control Agent Compose 范例使用同一个宿主机
`STARRY_PERSIST_ROOT` 实现这项拆分。

配置 `jwks.url` 后必须同时配置四个 JWKS mTLS identity 字段。客户端只信任
`jwks.ca_file`、主动提交配置的客户端证书、只允许 TLS 1.3，并要求 URL host 与
`jwks.server_name` 精确
一致。只有新文档完整通过校验并持久缓存，refresh 才整体替换 keyset；无效 refresh
保留 last-known-good keyset，但超过 `max_stale_seconds` 后验证 fail closed。远程刷新成功后
会写入 `<jwks.file>.metadata.json`，其中包含获取时间及精确 keyset 的 SHA-256。备份/恢复
JWKS 时必须同时保留该受限权限 sidecar；远程缓存若没有匹配 metadata，就无法证明新鲜度，
enforce 模式会在重启后拒绝使用。

配置 `introspection.url` 后，即使 `required: false` 也强制其四个 mTLS identity 字段；客户端
同样只信任配置 CA、只允许 TLS 1.3 并精确校验 server name。本地签名/claim 失败不发网络请求；请求使用
Kessoku 严格的仅 `token` JSON DTO，active response 必须包含与本地验证结果完全一致的
`sub`。本地有效 token 只按 SHA-256 hash 进入有界 cache。已配置 introspection 的 timeout、
TLS error、5xx、畸形 response、subject 缺失/不匹配或 inactive 均 deny/would-deny，不得
fail open。

## Audit 门禁与告警

从 Agent `GET /control/v1/status` 读取 `connection_auth`，至少记录：

- `configured_mode`、`effective_mode`、`verifier_state`、`key_count`、`key_age_seconds`；
- `attempts`、`allowed`、`denied`、`audit_would_deny`、`cache_hits`、
  `introspection_requests`、`introspection_failures` 的增量；
- client version、transport、request kind 与内部 reason 分布，但日志中不得保存 raw token、
  完整 JTI 或用户 secret。

verifier 非 `ready`、key 可能在下次成功 refresh 前过期、存在无法解释的 introspection
failure、或合法 client 产生意外 would-deny 时，禁止进入 enforce。enforce 中 `denied`/
`introspection_failures` 增长以及任何 last-known-good reload 被拒绝都应立即告警。

至少保持 audit 一个完整业务周期，并明确测试：

- missing、malformed、oversize、expired、future、错误 issuer/audience/scope、bad signature、
  unknown key 与 stale keyset；
- active、logout/revoked、disabled、deleted、password-reset 用户；
- current/previous key overlap、rotation、introspection outage 与 recovery；
- 支持的真实 client native P2P、native Relay、WSS/WSS 与两个 mixed 方向；
- 直接 `RequestRelay`、不存在 target 与 UDP no-allocation。

## Enforce 与紧急回滚

只有 audit 门禁通过后才 canary `mode: enforce`。`--must-login`（或 `MUST_LOGIN=Y`）构成
部署层 enforce floor；verifier 配置不完整时 startup/reload 会拒绝，而不会静默放行。

正常紧急回滚是在本机受控把 `enforce` 改为 `audit`，随后取得同步 activation ack。远程
Control API 故意没有特殊绕过或“一键关闭认证”。最长已签发 token lifetime 与 cache window
结束前必须同时保留 current/previous 公钥。
