# patch-v1.1.0 版本说明

[English](RELEASE-NOTES-patch-v1.1.0.md) | **简体中文**

patch-v1.1.0 为原生信令不可用的受限网络增加按需持久 WebSocket Signal，同时保留
原生信令和 P2P 作为日常更快的默认路径。

## 组件范围

- Starry 代码改动仅限 HBBS。
- 官方 HBBR 已提供原生 `21117/TCP` 与 `/ws/relay`；本 patch 只验证并使用该契约，不修改 HBBR。
- 不包含账户/API 服务；第三方 API 始终是独立的部署与安全选择。

## 新增功能

- 使用与原生注册相同 RegisterPk 身份校验的持久 `/ws/id` 注册。
- 单 writer WebSocket session、有限出站队列、generation-safe 路由替换、心跳、
  idle timeout、全局/单有效 IP session 限制和注册限速。
- WSS↔WSS 与 WSS↔原生信令。任一端使用 WSS 时，HBBS 明确强制 Relay，不会向受限端通告无法使用的 P2P 路径。
- 传输感知 Relay 选择：
  - `native` 要求官方 HBBS Relay 在线状态；
  - `wss` 要求证书有效且健康的 `/ws/relay`；
  - `mixed` 要求同一 Relay 同时满足两种状态。
- 每个 `wss://.../ws/relay` endpoint 都执行正常 DNS、TCP、TLS 证书链、域名和精确 WebSocket Upgrade 校验；不接受跳过 TLS、ping 或普通 HTTPS 200 替代。
- schema v2 严格验证：可信代理 CIDR、精确 Origin 白名单、endpoint 完整覆盖、帧/队列/session/限速与时序关系。
- 仅限回环地址的管理命令：
  - `websocket-status`（`ws`）；
  - `test-geo <IP_A> <IP_B> [native|wss|mixed]`（`tg`）。
- 使用真实 HBBS 进程、运行时测试 CA 和域名匹配证书的自动化测试，以及通过未修改官方 HBBR 的混合 Relay 流量测试。

## 行为与兼容性

- 现有 schema `version: 1` 继续有效。WebSocket Signal 保持关闭；v1 文档包含 `websocket_signal` 会被明确拒绝。
- 配置 WebSocket Signal 必须使用 schema `version: 2`。
- `websocket_signal.enabled` 不会更改任何客户端设置；每个客户端独立选择是否使用 WebSocket。
- 客户端关闭 WebSocket 时保留 patch-v1.0.0 原生/P2P 路径。
- 客户端开启 WebSocket 时使用 WSS 信令与仅 Relay 数据传输。
- 一端 WSS、一端原生时，通过不同传输进入同一 HBBR 节点和 Relay UUID。
- 启用 WebSocket Signal 时，每个 `relay_servers` 条目必须恰好对应一个 `relay_health.endpoints` 条目。
- 原生 RustDesk 客户端可以不发送 Origin；一旦存在 Origin，必须精确命中 `allowed_origins`。
- 只有直接 TCP peer 属于 `trusted_proxies` 时才接受转发客户端地址。

## 从 patch-v1.0.0 升级

1. 备份完整 HBBS 数据目录和当前 schema v1 文件。
2. 保留 v1 配置，先升级二进制或镜像；原生行为应保持不变。
3. 验证原生注册、登录后的 Secure TCP、P2P 与原生 Relay。
4. 部署证书有效的 Nginx `/ws/id` 和每台 Relay 的 `/ws/relay`。
5. 创建 endpoint 完整覆盖的 schema v2 文档，但先保持 `websocket_signal.enabled: false`。
6. 热加载并确认配置被接受。
7. 启用 WebSocket Signal，再次加载并查看 `websocket-status`。
8. 在扩大使用范围前，用真实客户端验证 WSS↔WSS 和两个方向的混合会话。

完整步骤和回滚门槛见
[`版本升级与回滚`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)。

## 回滚

回到 patch-v1.0.0 时：

1. 关闭受影响客户端的 WebSocket；
2. 恢复 schema v1 配置备份；
3. 恢复旧二进制或镜像；
4. 验证原生注册、Secure TCP 和真实远程控制会话。

patch-v1.0.0 不理解 schema v2。只恢复旧镜像而保留 v2 文档，会使 Starry 配置被拒绝，HBBS 回退到上游行为。

## 验证证据

发布门禁在官方 1.1.16 干净源码上重复应用 overlay，要求第二次应用无变化且
`git diff --check` 干净；同时运行锁定依赖的库测试与二进制检查、真实 HBBS
WebSocket 进程测试，以及通过未修改官方 HBBR 的双向原生/WSS 混合载荷测试。
Compose 和容器烟测覆盖两个发布架构。

发布后，维护者报告在已测试部署中正常使用，客户端可在原生和 WebSocket 模式之间
成功切换。这是该部署的运行报告，不是对其他 DNS、反向代理、证书、API、客户端或
网络环境的保证。

## 安全说明

- 在可信反向代理上使用有效证书终止公网 WSS。
- 不要公开 HBBS 管理入口。
- 将明文后端端口 `21118`、`21119` 限制到预期代理路径。
- 不要通过关闭 TLS 验证让 Relay 健康检查“通过”。
- 不要在文档和状态输出中包含完整 Peer ID、Token、原始客户端地址、API 秘密或 HBBS 私钥。

## 文档

- [容器镜像使用](CONTAINER.zh-CN.md)
- [配置参数参考](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)
- [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
- [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)
