# 常见问题排查

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Troubleshooting) | **简体中文**

按层排查：配置、进程、网络、TLS/代理、注册、Relay 分配，最后才是桌面数据。保留第一
个明确错误及其时间；后续反复重连常会增加干扰信息。

## 首轮只读信息

更改镜像、密钥、防火墙或客户端前先执行：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --since 15m hbbs hbbr
docker inspect rustdesk-starry-hbbs --format '{{json .State.Health}}'
ss -lntup | grep -E ':(21115|21116|21117|21118|21119)\b'
```

涉及 WebSocket 时：

```sh
sudo nginx -t
docker exec rustdesk-starry-hbbs sh -c \
  "echo '请改用已认证的 Control Agent GET /control/v1/status'"
```

记录精确镜像标签/摘要、配置摘要、客户端版本、两端公网网络、各自是否开启 WebSocket，
以及同一次连接的时间戳。共享输出前必须脱敏。

## 容器无法启动或始终 unhealthy

1. 阅读 HBBS 第一个错误，不要只看健康检查结果；
2. 确认绑定源存在且容器可写；
3. 检查主机网络中是否已有另一 HBBS 占用 `21115`、`21116` 或 `21118`；
4. 确认 `id_ed25519` 和 `id_ed25519.pub` 非空且相互匹配；
5. 用实际启动时的同一份 `.env` 渲染 Compose。

健康检查只证明密钥存在且本机 `21116/TCP` 接受连接，不能证明公网路由或客户端注册。

## 修改 YAML 后 Starry 功能消失

只要一个字段未知、重复、超范围或相互矛盾，Starry 就会拒绝整份候选配置，并保持
上游兼容行为，不会部分应用。

检查：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "test -s /starry/config.yaml"
docker restart rustdesk-starry-hbbs
docker logs --tail 200 rustdesk-starry-hbbs
```

修复日志明确指出的字段，不要随机删除无关部分。结构 `version: 1` 不能包含
`websocket_signal`；需要此部分时改用 `version: 2`。

## 客户端显示“未就绪”或无法注册

从客户端到 HBBS 逐项检查：

- ID Server 必须解析到预期 HBBS；原生路径需要 `21116/TCP` 和 `21116/UDP`；
- 客户端公钥必须与 `id_ed25519.pub` 完全一致，删除复制粘贴带入的空白；
- API Server 不是 ID Server。网页/API 成功不能说明原生 HBBS 可达；
- 涉及 TLS 或 Token 过期时，检查客户端和服务器时间；
- 换一个网络测试，以区分本地防火墙和服务端问题。

若只开放 TCP，被控端仍无法完成原生注册是官方 1.1.16 的预期行为：`RegisterPk`/心跳走
UDP，TCP listener 对该注册消息返回 `NOT_SUPPORT`。这与控制端通过 TCP/Secure TCP 发起
认证连接是两条不同路径。不能开放 UDP 时，应验证 WSS `/ws/id` 和 `/ws/relay`，而不是
把原生注册失败误判为 JWT 拒绝。

不要用“重新生成服务端密钥”测试连通性，这会改变所有客户端信任的身份。

## API 登录成功但控制超时

把它们视为三层：

1. API 认证和账户数据；
2. 原生 HBBS `21116/TCP` 信令，包括发生协商时的 Secure TCP；
3. P2P 或 HBBR 数据传输。

先确认客户端 ID Server 和公钥，再关联 HBBS 握手日志。会话需要 Relay 时，还要单独
确认 `21117/TCP` 和已选 HBBR 日志。更改 API `session_id` 或反向代理路由不能修复
失败的原生 HBBS 握手。

## `Failed to secure tcp`

依次检查：

1. Starry 配置被接受，且 `secure_tcp.mode` 为 `auto`；
2. 客户端使用正确 HBBS 公钥；
3. `21116/TCP` 到达 Starry HBBS，而不是旧进程或重复实例；
4. 客户端和服务端版本/日志属于同一次尝试；
5. 没有四层代理截断或改写连接。

Secure TCP 是原生 HBBS 信令，不是 WSS，也不是 API HTTPS。排查这一层时不要关闭证书
校验或修改 HBBR。

## Geo 选错 Relay

按优先链检查：

1. 客户端 Relay Server 字段为空；
2. HBBS 观测到的公网 IP 与测试一致；
3. 所需 MMDB 已读取，且含预期记录；
4. 具体规则位于兜底规则之前；
5. `symmetric` 符合预期方向；
6. Relay 拼写与 `relay_servers` 条目一致；
7. 首选 Relay 符合当前传输要求。

Relay 列表是严格优先级；第一台健康 Relay 被反复选择是正常结果，不是负载均衡失败。

## `test-geo` 返回 `""`

空结果表示按传输要求过滤后没有符合条件的 Relay，可能是：

- `native` 没有原生在线 Relay；
- `wss` 没有证书有效且健康的 endpoint；
- `mixed` 没有同时原生在线与 WSS 健康的同一台 Relay；
- 规则匹配但其中 Relay 不可用，且后续规则也不能选择；
- 实际 Relay 池为空。

对比 `relay-servers`、`websocket-status` 和三种 `test-geo` 模式。
`reload-geo` 只重载 Geo/MMDB；修改 Relay、WebSocket 或其他结构字段后应执行
已认证的 Control Agent plan/apply 或 runtime-reload 操作。

## MMDB 下载或查询失败

在服务器上检查配置 URL，但不要输出凭据。常见原因包括：下载到 HTML 登录/许可证
页面、签名 URL 过期、TLS 被拦截、文件小于 `minimum_bytes`、缺少 MaxMind 标记，或
数据库不支持规则请求的记录类型。

替换校验失败时 Starry 会保留上一个可读数据库。确认文件时间和启动/重载消息后，
再判断新数据是否生效。`force_update: true` 是临时恢复工具，不应永久开启。

## WSS 返回 404、400 或 502

| 响应 | 常见含义 |
| --- | --- |
| `404` | 域名/路径错误、Nginx 不是精确 location，或请求进入了另一个虚拟主机。 |
| `400` | 缺少 Upgrade 请求头或 HTTP/1.1、Origin 无效，或直连后端请求格式错误。 |
| `502` | Nginx 无法访问 `/ws/id` 的 `127.0.0.1:21118` 或 `/ws/relay` 的 `127.0.0.1:21119`。 |

确认精确路径、后端监听、Nginx 错误日志和 `proxy_http_version 1.1`。配置的健康 endpoint
不能添加尾斜杠、查询或不同路径。

## WSS 返回 101，但 RustDesk 仍失败

`101` 只证明 HTTP Upgrade。继续检查 RustDesk 协议层：

- schema v2 或 v3 和 `websocket_signal.enabled: true` 已被接受；
- 客户端通过预期 ID Server 域名使用 `/ws/id`；
- Upgrade 后的注册和路由日志；
- 证书有效的 `/ws/relay` endpoint 处于健康状态；
- 混合会话有同一台符合要求的 Relay；
- 两端客户端和 HBBR 同一次尝试的关联日志。

Upgrade 成功但注册失败时，继续反复更改 DNS 或 TLS 不能解决协议错误。

## 转发 IP 或 Origin 被拒绝

只有直连 Peer 位于 `trusted_proxies` 时，HBBS 才信任转发 IP 请求头。先确定 HBBS
实际看到的源地址，再加入最小且正确的代理 CIDR，绝不能信任所有地址。

客户端发送 `Origin` 时必须与 `allowed_origins` 中一项完全相等；空列表会拒绝所有
携带 Origin 的请求，但不发送该请求头的原生客户端仍可接入。协议、主机和端口都
属于 Origin；路径、凭据、查询和片段不是有效配置项。

## 原生正常，WSS 不健康

原生 HBBR 在线状态和 WSS endpoint 健康状态有意分离。针对 `websocket-status` 中的
Relay 域名检查 DNS、公网 `443`、证书链、主机名/SNI、精确 `/ws/relay`、Nginx 到
`21119` 的可达性以及成功/失败阈值。Ping 和普通 HTTPS `200` 均不足以证明健康。

## 混合模式没有 Relay

Mixed 要求**同一个 Relay 名称**既原生在线，又 WSS 健康。检查：

- `relay_servers` 与 `relay_health.endpoints[].relay` 使用完全相同的 `host:21117`；
- 发布物中保留上游中继数据路径的 HBBR 将该值报告为在线；
- 当前配置 generation 中对应 WSS endpoint 健康；
- Geo 规则列出该 Relay。

不要把不同物理节点的原生名称和 WSS 名称强行映射；它们应描述同一个、可通过两种
路径访问的 HBBR 服务。

## 会话连接但很慢

先从客户端和服务端日志判断是 P2P 还是 Relay。Relay 会话按 UUID/session 关联两条
链路，再测量客户端到 Relay 的延迟、丢包、吞吐，以及节点 CPU、内存和网络限速。

地理距离更近不保证延迟更低，应按真实测量调整有序规则。只重启或重载确实发生状态
变化的组件，再重复同一受控传输。

## 已启用 FastCompat 但没有授权

tag 64 为空是安全 fallback，不代表 Relay 失败。读取
`/control/v1/relays.fast_relay`，确认哪项计数增加：

- `disabled` 或 `invalid_configuration`：检查当前 schema v4 策略；
- `insecure_requests`：必须取得鉴权严格 `allow`，并使用 Secure TCP 或受保护 WSS 信令；
- `quality_selection_failures`：`RequestRelay` 前先完成支持质量协议的 PunchHole 预检与最终
  来源绑定选择；
- `missing_signing_keys` 或 `signing_failures`：检查保留的 HBBS 身份/密钥和系统时钟，但不
  输出私密材料；
- `rate_limited`：停止重连/签名风暴；
- `response_misses`：检查目标 IP 绑定、UUID、最终 Relay、有效期和配置 generation。

不要把签名授权复制到 Issue，也不要让 Kessoku 代理它。客户端必须使用现有 HBBS 公钥
验签，并拒绝过期、UUID 错误或畸形载荷。本版出现 `allow_fast_media_v1: false` 是预期。

## Profile 切换不提交或删除了错误 route

除非成功 Ready ACK 精确回显 pending activation ID/epoch，并携带 32 字节 lease 与非零
generation，否则 Akari 必须保持原 Profile 已提交。全默认值 ACK 通常表示目标服务端是
legacy，不能放宽客户端检查。

把 `/control/v1/relays.profile_activation` 作为聚合证据：

- `invalid_requests`：检查精确长度和必须非零字段；
- `stale_rejections`：检查 epoch 复用、错误节点 lease、延迟 A/B 心跳/注销或旧 WSS
  reader；
- `rate_limited`：停止切换循环；精确 identity 在 30 秒内最多新签发 12 个 lease，普通
  IP/global 防护仍启用；
- `capacity_rejections`：停止发布，排查无界不同 peer/identity churn；
- `disconnect_cleanups`：精确 reader/socket 关闭后增长属于预期；
- `ttl_expirations`：只能偶发用于崩溃兜底；持续增长说明 disconnect/deactivation 送达失败。

多 HBBS 节点必须按 `instance.id` 对比 capability/计数，并使用该节点 lease 调用
`/peers:verify`。不得把 lease 复制到另一节点，也不得在诊断中输出 activation ID、public
key 或 lease。回滚前遵循
[Profile Activation Lease v1](../../reference/PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)。

## 连接认证意外拒绝或放行

从 Control Agent status 读取 `configured_mode`、`effective_mode`、`verifier_state`、key
age 与 metric 增量。即使文档为 audit/off，`--must-login` 也可能使 effective mode 成为
`enforce`。本地 claim/signature 错误不得调用 introspection；其他本地有效请求会调用已配置
introspection，并在 timeout、TLS、5xx、畸形或 inactive response 时 fail closed。

检查精确 `typ=at+jwt`、issuer/audience/token-use/scope、`sub == user_id`、时钟同步、显式 `kid`、
Ed25519 公钥轮换 overlap、JWKS stale、client token placement、request kind 与 transport。
不得把 raw token 粘贴到 Issue。合法 client 回归时，通过本机 acknowledged reload 把
enforce 改回 audit，不能增加远程 bypass。

若 JWKS endpoint 的服务端 idle timeout 较短，旧 keep-alive 连接可能表现为间歇性 refresh
失败。patch-v1.2.0 将内部 mTLS HTTP pool 的 idle lifetime 限制为 15 秒；仍出现失败时同时
检查 Starry refresh 日志、Kessoku idle timeout、证书链、server name 与
`key_age_seconds`，不得通过增大 `max_stale_seconds` 掩盖持续故障。

## Control Agent 不可用或阻断写入

分别检查 TLS handshake、URI-SAN allowlist、service-JWT audience/azp/scope 与本机 HBBS
连通。`write_enabled: false` 时写 endpoint 返回 404 是预期结果。ETag/plan/idempotency
冲突是并发保护，不能通过移除 precondition 重试。

operation 进入 `manual_intervention_required` 时停止重试。保留 state/audit/recovery
directory，对比 managed config 精确 bytes 与 HBBS runtime generation/digest，恢复经过审核
的 last-known-good 文件并执行本机 acknowledged reload。按
[Control Agent 恢复 runbook](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)
处理；为清除阻断而删除 state 会破坏必要证据。

## 升级后回归

首次出现可重现协议回归时停止扩大部署，保留新旧镜像摘要、配置和日志。如果安全，
先关闭新功能；否则按[版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)
恢复旧的不可变标签和配置。

不要用空备份覆盖数据目录，也不要重新生成密钥。
