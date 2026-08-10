# rustdesk-server-starry

[English](README.md) | **简体中文**

## 项目介绍

`rustdesk-server-starry` 是官方
[`rustdesk/rustdesk-server`](https://github.com/rustdesk/rustdesk-server)
的 HBBS 专用 overlay 扩展。它在不长期维护一份分叉上游源码的前提下，增加基于策略的
Relay 分配与登录客户端所需的传输兼容能力。

每次构建都从精确锁定的官方 RustDesk Server 版本开始。
[`scripts/apply_overlay.py`](scripts/apply_overlay.py)
会验证固定源码锚点并注入 Starry 模块；在同一份源码上第二次执行必须不再产生变化。
一旦上游结构变化导致锚点或测试失效，发布流程会停止，而不会生成只注入了部分功能的服务端。

Starry 为 **HBBS** 扩展以下能力：

- 根据连接双方的国家、城市、子区域、ASN 和运营商信息，严格按顺序选择 Relay；
- MMDB 下载、替换、保留与定期更新前的完整校验；
- 在原生 HBBS TCP `21116` 上提供 RustDesk 客户端兼容的 Secure TCP 协商与加密传输；
- 可选的 `/ws/id` 持久 WebSocket 信令，包括 WSS↔WSS 与 WSS↔原生 Relay 会话；
- 验证证书的 `/ws/relay` 健康过滤，以及本机状态查看和规则测试命令。

项目有明确的组件边界：

| 组件 | Starry 范围 | 部署责任 |
| --- | --- | --- |
| HBBS | 由 overlay 修改 | 运行 Starry 的 `hbbs` 二进制或镜像命令。 |
| HBBR | 不做任何修改 | 使用官方 HBBR。为了方便，Starry 发布物可能附带从同一锁定上游版本构建、未经修改的 HBBR。 |
| 账户/API 服务 | 不包含 | 账户登录、地址簿、设备数据和管理功能需要部署者自行选择独立 API 实现。 |

API 登录、HBBS 信令连接与 HBBR 数据转发是三个独立协议层。Starry 不会把第三方 API
变成 Relay，不会 fork HBBR，也不会替代 RustDesk 客户端。

当前 overlay 版本：**patch-v1.1.0**。参见
[`patch-v1.1.0` 版本说明](RELEASE-NOTES-patch-v1.1.0.zh-CN.md)和
[`更新日志`](CHANGELOG.zh-CN.md)。Docker 镜像发布于
[`ghcr.io/q1ngyang/rustdesk-server-starry`](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)。

> 这是非官方社区项目，与 RustDesk、MaxMind、任何 MMDB 镜像提供方或任何 AI
> 服务提供方均无隶属或背书关系。镜像不内置 GeoLite2 数据库；部署者应自行选择合法、
> 可信的数据源并遵守相应许可证。项目部分代码与文档使用 AI 辅助生成或修订；这些内容
> 仍受同一项目许可证约束，且不附带任何额外保证。

## 文档目录

英文是默认文档语言；每篇说明性文档均提供简体中文版本。可直接执行的配置和编排文件由
两种语言共同引用，避免两套示例发生功能漂移。

| 文档 | 用途 |
| --- | --- |
| [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started) | 选择部署方式并完成第一组经过验证的客户端连接。 |
| [Docker 镜像使用](CONTAINER.zh-CN.md) | 拉取、检查、运行、固定、升级和排查 GHCR 镜像。 |
| [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment) | 推荐的单机 Docker Compose 部署。 |
| [原生部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Native-Deployment) | DEB、Linux 二进制、systemd 与 Windows 部署。 |
| [多节点部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment) | 中心 HBBS、官方 HBBR 节点和可选第三方 API。 |
| [反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS) | `/ws/id`、`/ws/relay`、API、证书和防火墙要求。 |
| [客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration) | ID Server、API Server、公钥、Relay 字段与 WebSocket 开关。 |
| [配置参数参考](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference) | schema 字段、默认值、范围、依赖和回退行为。 |
| [GEO 规则入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics) | 国家规则、优先级、对称匹配、回退与 `test-geo`。 |
| [GEO 规则进阶](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Advanced) | 城市、ASN、ISP、嵌套表达式、引号与设计模式。 |
| [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification) | 从静态检查到真实桌面会话的分层验收。 |
| [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting) | 配置、MMDB、Secure TCP、WSS、HBBR、API 与升级故障。 |
| [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback) | 备份、迁移 schema、验证版本和安全恢复。 |
| [架构与构建](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Architecture-and-Build) | overlay 机制、协议边界、测试与自动发布。 |
| [English documentation](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Home) | English documentation home. |

## 许可证

官方 RustDesk Server 源码与 Starry overlay 均按 GNU Affero General Public
License v3.0 发布。二进制和镜像由对应的锁定上游版本加本 overlay 构建；详见
[`LICENSE`](LICENSE)。
