# rustdesk-server-starry 文档

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Home) | **简体中文**

`rustdesk-server-starry` 以锁定版本的官方 RustDesk Server 源码为基础，扩展 HBBS 的
中继服务器选择与连接能力。主要功能包括：按地理位置依次选择中继服务器、MMDB 管理、
安全 TCP、可选 WebSocket 信令、连接认证、安全的配置生效机制、中继分配模拟，以及
可选的 Linux 最小权限管理代理。容器镜像还包含从**同一上游版本**构建的 HBBR；其中继
数据路径保持不变，仅在 WebSocket 握手中增加有界版本响应头，以便盘点实际运行版本。

## 首先理解组件边界

| 组件 | 职责 | Starry 是否提供或修改？ |
| --- | --- | --- |
| Starry HBBS | 注册设备、协调连接、协商安全 TCP、执行地理位置规则并选择中继服务器。 | **经过 Starry 修改**。 |
| Starry 镜像内的 HBBR | P2P 不可用或使用 WebSocket 时转发远程控制数据。 | 上游中继数据路径不变，仅增加 WebSocket 握手版本响应头；示例与 HBBS 使用同一锁定镜像防止版本漂移。 |
| Starry 管理代理 | 通过 mTLS 和按权限划分的服务令牌管理一台本机 HBBS。 | 可选 Linux 组件；默认禁止写入配置。 |
| 账户/API 服务 | 登录、地址簿、设备数据和管理。 | **不包含**。可搭配兼容的第三方 API，推荐 Kessoku。 |
| RustDesk 客户端 | 向 HBBS 注册并建立 P2P 或中继会话。 | 不包含。 |

API 登录成功不能证明 HBBS 信令传输正常；HBBS 注册成功也不能证明 HBBR 数据能够
传输。本文档始终分层描述这些组件，避免把局部检查当成完整成功。

## Starry 增加的能力

- 使用两端公网地址信息严格按顺序选择 Relay。
- 支持国家、洲、子区域、城市、GeoNames ID、ASN 和 ISP 匹配。
- 定时下载 MMDB，并在替换前校验、失败时保留最后可用版本。
- 原生 HBBS `21116/TCP` 上的客户端兼容 Secure TCP。
- 面向受限网络的可选持久 `/ws/id` 信令。
- 通过保持上游中继数据路径的 HBBR 实现 WSS↔WSS 与 WSS↔原生会话。
- 检查 `/ws/relay` 的证书与可用状态，只把可用中继服务器分配给 WSS 客户端。
- 仅在所有子系统确认成功后启用新配置；失败时保留最近一次有效配置。
- 在原生 TCP、安全 TCP 和 WSS 上提供可选连接令牌认证；UDP 不能发起此类连接。
- 提供中继状态快照和不影响线上状态的分配模拟。
- 提供仅限本机的控制协议，以及可选的 mTLS/RBAC 管理代理。

## 选择入口

| 你的情况 | 从这里开始 |
| --- | --- |
| 第一次部署或 Docker 经验较少 | [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started) |
| 从 GHCR 包页面进入 | [Docker 镜像使用](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Image-Usage) |
| 一台 Linux 服务器，没有独立 Relay | [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment) |
| 已有 systemd 或 Windows 环境 | [原生部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Native-Deployment) |
| 一个中心和多台 HBBR | [多节点部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment) |
| 需要 WebSocket | [反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS) |
| 需要账户、登录或 API | [账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration) |
| 正在把登录接入 HBBS 连接授权 | [连接认证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication) |
| 需要 Relay 可见性或受管配置事务 | [Control Agent](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent) |
| 服务已运行但功能异常 | [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting) |
| 准备修改版本 | [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback) |

多数用户推荐在 Linux 主机上使用 Docker Compose。它提供可重复的服务定义、明确的
持久化数据和清晰的回滚点。

## 推荐阅读顺序

1. [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)；
2. 对应部署方式页面；
3. [客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration)；
4. 需要账户功能时阅读[账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration)；
5. [配置参数详解](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)；
6. 需要对应功能时阅读[连接认证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication)或[管理代理](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)；
7. [地理位置规则入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics)；
8. [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)；
9. 出现故障时根据[常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)收集证据并定位原因。

## 安全默认值

- 生产环境固定发布版本。
- 由 HBBS 动态分配 Geo Relay 时，客户端 Relay Server 字段保持为空。
- 当前网络不需要时，客户端保持 WebSocket 关闭。
- 所有 Relay 都具有有效 `/ws/relay` 前，不启用 WebSocket Signal。
- 不绕过 TLS 验证。
- 审计模式的日志核对完成前保持连接认证关闭；`audit` 只记录，不会拦截连接。
- 管理代理先按只读模式接入，并保持 HBBS 的本地管理通道不对公网开放。
- 只分发 `id_ed25519.pub`；私钥 `id_ed25519` 必须保密并备份。
- Compose 校验、端口开放或 HTTP 101 都只是局部证据，不是桌面控制成功。

## 项目与法律状态

这是非官方社区项目，与 RustDesk、MaxMind、任何 MMDB 镜像提供方或任何 AI 服务
提供方均无隶属关系。镜像不内置 GeoLite2 数据库。部分代码与文档使用 AI 辅助生成
或修订，不附带任何额外保证。

源码：<https://github.com/q1ngyang/rustdesk-server-starry>

许可证：AGPL-3.0
