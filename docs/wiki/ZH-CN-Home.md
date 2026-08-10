# rustdesk-server-starry 文档

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Home) | **简体中文**

`rustdesk-server-starry` 是官方 RustDesk Server 的 HBBS 专用 overlay。它以
官方源码为构建基础，增加有序 Geo Relay 选择、受管理的 MMDB、Secure TCP 兼容和
可选 WebSocket 信令。

## 首先理解组件边界

| 组件 | 职责 | Starry 是否提供或修改？ |
| --- | --- | --- |
| Starry HBBS | 注册设备、协调连接、协商 Secure TCP、计算 Geo 规则并选择 Relay。 | overlay **会修改**。 |
| 官方 HBBR | P2P 不可用或使用 WebSocket 时转发远程控制数据。 | **不修改**。使用官方 HBBR，或 Starry 产物中从上游构建的未修改版本。 |
| 账户/API 服务 | 登录、地址簿、设备数据和管理。 | **不包含**。需要时独立选择并加固。 |
| RustDesk 客户端 | 向 HBBS 注册并建立 P2P 或 Relay 会话。 | 不包含。 |

API 登录成功不能证明 HBBS 信令传输正常；HBBS 注册成功也不能证明 HBBR 数据能够
传输。本文档始终分层描述这些组件，避免把局部检查当成完整成功。

## Starry 增加的能力

- 使用两端公网地址信息严格按顺序选择 Relay。
- 支持国家、洲、子区域、城市、GeoNames ID、ASN 和 ISP 匹配。
- 定时下载 MMDB，并在替换前校验、失败时保留最后可用版本。
- 原生 HBBS `21116/TCP` 上的客户端兼容 Secure TCP。
- 面向受限网络的可选持久 `/ws/id` 信令。
- 通过未修改官方 HBBR 实现 WSS↔WSS 与 WSS↔原生会话。
- 供 WSS 和 mixed 分配使用的证书验证 `/ws/relay` 健康状态。
- 本机配置重载、状态、Relay 列表和规则测试命令。

## 选择入口

| 你的情况 | 从这里开始 |
| --- | --- |
| 第一次部署或 Docker 经验较少 | [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started) |
| 从 GHCR 包页面进入 | [Docker 镜像使用](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Image-Usage) |
| 一台 Linux 服务器，没有独立 Relay | [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment) |
| 已有 systemd 或 Windows 环境 | [原生部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Native-Deployment) |
| 一个中心和多台 HBBR | [多节点部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment) |
| 需要 WebSocket | [反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS) |
| 服务已运行但功能异常 | [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting) |
| 准备修改版本 | [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback) |

多数用户推荐在 Linux 主机上使用 Docker Compose。它提供可重复的服务定义、明确的
持久化数据和清晰的回滚点。

## 推荐阅读顺序

1. [快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)；
2. 对应部署方式页面；
3. [客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration)；
4. [配置参数参考](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)；
5. [GEO 规则入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics)；
6. [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)；
7. 证据显示失败后再进入[常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)。

## 安全默认值

- 生产环境固定发布版本。
- 由 HBBS 动态分配 Geo Relay 时，客户端 Relay Server 字段保持为空。
- 当前网络不需要时，客户端保持 WebSocket 关闭。
- 所有 Relay 都具有有效 `/ws/relay` 前，不启用 WebSocket Signal。
- 不绕过 TLS 验证。
- 只分发 `id_ed25519.pub`；私钥 `id_ed25519` 必须保密并备份。
- Compose 校验、端口开放或 HTTP 101 都只是局部证据，不是桌面控制成功。

## 项目与法律状态

这是非官方社区项目，与 RustDesk、MaxMind、任何 MMDB 镜像提供方或任何 AI 服务
提供方均无隶属关系。镜像不内置 GeoLite2 数据库。部分代码与文档使用 AI 辅助生成
或修订，不附带任何额外保证。

源码：<https://github.com/q1ngyang/rustdesk-server-starry>

许可证：AGPL-3.0
