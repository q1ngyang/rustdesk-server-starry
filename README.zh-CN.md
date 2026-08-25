# rustdesk-server-starry

[English](README.md) | **简体中文**

## 项目介绍

`rustdesk-server-starry` 是官方
[`rustdesk/rustdesk-server`](https://github.com/rustdesk/rustdesk-server)
的 HBBS 扩展层。它不长期维护一份与上游分离的源码副本，而是在锁定版本的官方源码上
加入按规则选择中继服务器和登录客户端所需的传输兼容能力。

每次构建都从精确锁定的官方 RustDesk Server 版本开始。
[`scripts/apply_overlay.py`](scripts/apply_overlay.py)
会验证固定源码锚点并注入 Starry 模块；在同一份源码上第二次执行必须不再产生变化。
一旦上游结构变化导致锚点或测试失效，发布流程会停止，而不会生成只注入了部分功能的服务端。

Starry 为 **HBBS** 增加以下能力：

- 根据连接双方的国家、城市、行政区、ASN 和运营商信息，严格按顺序选择中继服务器；
- 自动下载、检查、替换和定期更新 MMDB；更新失败时保留上一份可用数据库；
- 在 HBBS 原生 `21116/TCP` 端口上提供兼容 RustDesk 客户端的安全 TCP 协商与加密传输；
- 可选的 `/ws/id` 持久 WebSocket 信令，包括 WSS↔WSS 与 WSS↔原生中继会话；
- 验证 `/ws/relay` 的证书和可用状态；
- 配置结构版本 3：只有所有相关子系统确认成功后才启用新配置，失败时保留最近一次有效配置；
- 对原生 TCP、Secure TCP 和 WSS 上的 `PunchHoleRequest` 与直接
  `RequestRelay` 提供可选的 Ed25519 连接令牌认证；UDP 不支持发起这种已认证连接；
- 提供只读中继状态快照、不影响线上状态的分配模拟，以及由 mTLS、按权限划分的服务
  令牌和原子配置事务保护的独立 Linux 管理代理。

项目有明确的组件边界：

| 组件 | Starry 范围 | 部署责任 |
| --- | --- | --- |
| HBBS | 由 Starry 扩展层修改 | 运行 Starry 镜像或发布物中的 `hbbs`。 |
| HBBR | 不做任何修改 | 使用同一 Starry 镜像或发布物中附带的 HBBR。它与 HBBS 来自同一锁定上游版本，可避免分别更新。 |
| 管理代理 | 可选 Starry 组件；v1.2 仅支持 Linux | HBBS 本地控制保持仅本机可用；管理代理只通过私有 mTLS 管理通道访问，默认禁止写入配置。 |
| 账户/API 服务 | 不包含 | 可搭配兼容的第三方 API；推荐同一开发者维护的 Kessoku。 |

账户 API 登录、HBBS 信令连接、可选的 Starry 管理接口与 HBBR 数据转发是相互独立的
协议层。Starry 不会让第三方 API 代替中继服务器，不会修改 HBBR，也不会替代 RustDesk 客户端。

当前版本：**patch-v1.2.0**。参见
[`patch-v1.2.0` 版本说明](RELEASE-NOTES-patch-v1.2.0.zh-CN.md)和
[`更新日志`](CHANGELOG.zh-CN.md)。Docker 镜像发布于
[`ghcr.io/q1ngyang/rustdesk-server-starry`](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)。

patch-v1.2.0 正式提供 Docker `linux/amd64` 镜像、Linux x86_64 二进制文件和 amd64 DEB
安装包。ARM 只尽力保持源码兼容，Windows 只有实验性构建检查；两者都不属于 v1.2.0
正式发布文件。

> 这是非官方社区项目，与 RustDesk、MaxMind、任何 MMDB 镜像提供方或任何 AI
> 服务提供方均无隶属或背书关系。镜像不内置 GeoLite2 数据库；部署者应自行选择合法、
> 可信的数据源并遵守相应许可证。项目部分代码与文档使用 AI 辅助生成或修订；这些内容
> 仍受同一项目许可证约束，且不附带任何额外保证。

## 文档目录

英文是默认文档语言；每篇说明性文档均提供简体中文版本。可直接执行的配置和编排文件由
两种语言共同引用，避免两套示例发生功能漂移。

| 文档 | 用途 |
| --- | --- |
| [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started) | 从域名、防火墙到地理位置规则、WSS 和真实客户端验收的单机 Docker 完整教程。 |
| [Docker 镜像使用](CONTAINER.zh-CN.md) | 拉取、检查、运行、固定、升级和排查 GHCR 镜像。 |
| [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment) | 推荐的单机 Docker Compose 部署。 |
| [原生部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Native-Deployment) | 正式 amd64 DEB/Linux 部署与非阻断兼容说明。 |
| [多节点部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment) | 中心 HBBS、使用同一固定镜像版本的 HBBR 节点和可选账户服务。 |
| [反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS) | `/ws/id`、`/ws/relay`、API、证书和防火墙要求。 |
| [客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration) | ID 服务器、API 服务器、服务器公钥、中继服务器字段和 WebSocket 开关。 |
| [账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration) | 第三方 API 兼容范围、推荐 Kessoku、组件边界和安全接入顺序。 |
| [配置参数详解](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference) | 所有配置项、默认值、范围、依赖和失败时的处理方式。 |
| [连接认证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication) | 令牌要求、从仅记录到强制拦截的上线顺序、传输范围、故障处理与回滚。 |
| [管理代理](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent) | Linux 部署、mTLS/服务令牌授权、只读模式、配置事务、恢复与接口约定。 |
| [地理位置规则入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics) | 国家规则、优先级、双向匹配、兜底规则与 `test-geo`。 |
| [地理位置规则进阶](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Advanced) | 城市、ASN、运营商、嵌套表达式、引号和常用写法。 |
| [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification) | 从静态检查到真实桌面会话的分层验收。 |
| [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting) | 配置、MMDB、安全 TCP、WSS、HBBR、API 和升级故障。 |
| [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback) | 备份、迁移配置结构、验证版本和安全恢复。 |
| [架构与构建](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Architecture-and-Build) | 扩展层机制、协议边界、测试与自动发布。 |
| [English documentation](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Home) | English documentation home. |

## 许可证

官方 RustDesk Server 源码与 Starry 扩展层均按 GNU Affero General Public
License v3.0 发布。二进制和镜像由对应的锁定上游版本加本扩展层构建；详见
[`LICENSE`](LICENSE)。
