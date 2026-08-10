# rustdesk-server-starry 容器镜像

[English](CONTAINER.md) | **简体中文**

本文面向从 GHCR 包页面进入、需要直接部署镜像的用户。项目介绍和源码关系见主
[`README.zh-CN.md`](README.zh-CN.md)。

## 镜像包含什么

多架构镜像支持 `linux/amd64` 与 `linux/arm64`，由一个锁定的官方 RustDesk
Server 版本加 Starry HBBS overlay 构建。

| 命令 | 来源 | 用途 |
| --- | --- | --- |
| `hbbs` | 官方 HBBS 加 Starry overlay | ID、会合、信令、Secure TCP、Geo Relay 选择和可选 WebSocket Signal。 |
| `hbbr` | 未修改的上游 HBBR | 从相同上游版本构建的便捷副本。推荐示例使用官方 RustDesk Server 镜像运行 HBBR，使组件边界保持清晰。 |
| `rustdesk-utils` | 未修改的上游工具 | 密钥和数据库维护工具。 |

镜像**不包含**账户/API 服务，也不包含任何 GeoLite2/MMDB 数据库。请独立选择这些
组件、审查其许可证，并将秘密保留在镜像之外。

## 选择标签

发布标签格式为：

```text
<官方-rustdesk-server-版本>-patch-vX.Y.Z
```

例如：

```text
1.1.16-patch-v1.1.0
```

- 日常生产部署使用不可变版本标签。
- 必须完全锁定 manifest 时使用镜像 digest。
- `latest` 跟随最近一次成功发布的 Starry 版本，仅适合评估，或已建立自动更新与
  回滚流程的运维者。

拉取当前文档对应版本：

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0
```

公开 GHCR 镜像可匿名拉取。上线前检查实际 digest 和平台：

```sh
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  --format '{{json .RepoDigests}}'

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0
```

## 推荐快速部署

仓库的 [`examples/compose.yaml`](examples/compose.yaml) 会启动：

- 本镜像中的 Starry HBBS；
- 与上游版本匹配、未经修改的官方 HBBR。

在 Linux Docker 主机上执行：

```sh
mkdir -p /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLO \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.yaml
curl -fsSLo .env \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.env.example

mkdir -p data
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
```

使用仓库 checkout 时：

```sh
cp examples/.env.example .env
cp examples/compose.yaml compose.yaml
mkdir -p data
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
```

示例使用 Linux host 网络，使 HBBS 能看到真实对端地址，并直接在主机监听 RustDesk
标准端口。它不是 Docker Desktop 示例。

## 持久化数据与首次启动

两个服务把同一主机数据目录挂载到 `/root`。该目录包含服务器身份、SQLite 状态、
日志、下载的 MMDB 和 Starry 配置，应当整体备份。

Starry HBBS 首次启动会创建：

```text
data/
├── id_ed25519
├── id_ed25519.pub
└── starry/
    ├── config.yaml
    └── config.example.yaml
```

`starry/config.yaml` 初始为空，这是有意设计。空配置或无效配置会禁用整份 Starry
配置，使 HBBS 使用上游命令行行为。请从 `starry/config.example.yaml` 复制需要的
部分，或从 [`config/`](config/) 中选择入门模板。

绝不要把 `id_ed25519` 复制到纯 Relay 节点或文档中。公钥 `id_ed25519.pub` 是客户端
需要配置的 Key，可以分发。

## 端口

| 端口 | 协议 | 进程 | 用途 |
| --- | --- | --- | --- |
| `21115` | TCP | HBBS | NAT 测试和仅限回环地址的管理命令；不要公开代理管理入口。 |
| `21116` | TCP | HBBS | 原生信令、打洞协调与 Secure TCP。 |
| `21116` | UDP | HBBS | 注册与心跳。 |
| `21117` | TCP | HBBR | 原生 Relay 数据。 |
| `21118` | TCP | HBBS | `/ws/id` 明文 WebSocket 后端；只允许可信反向代理访问。 |
| `21119` | TCP | HBBR | `/ws/relay` 明文 WebSocket 后端；只允许可信反向代理访问。 |
| `443` | TCP | Nginx | 使用 WebSocket 或 HTTPS API 时的公网 TLS/WSS 入口。 |

只开放部署所需入口。WebSocket 客户端必须同时具备证书有效的 `/ws/id` 和
`/ws/relay`；只发布其中一条路径不构成完整 WebSocket 部署。

## 配置 Starry 功能

编辑主机上生成的文件：

```sh
vi data/starry/config.yaml
```

无需替换进程即可热加载：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
```

若返回错误，整份新 Starry 配置都不会生效，旧 Starry 状态也不会保留；HBBS 会恢复
上游行为。恢复或修正文件后再次加载；不要因为容器已重启就假设无效配置已经启用。

可从以下模板开始：

- [`config/config.minimal.yaml`](config/config.minimal.yaml)：仅启用 Secure TCP；
- [`config/config.geo-basic.yaml`](config/config.geo-basic.yaml)：按国家有序选择 Relay；
- [`config/config.geo-advanced.yaml`](config/config.geo-advanced.yaml)：嵌套城市/ASN/ISP 规则；
- [`config/config.websocket.yaml`](config/config.websocket.yaml)：schema v2 WebSocket Signal 与证书验证的 Relay 健康检查。

## 不使用 Compose 执行命令

长期服务推荐 Compose。临时检查可执行：

```sh
docker run --rm \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  hbbs --help
```

手动启动持久化 HBBS：

```sh
mkdir -p /opt/rustdesk-server-starry/data

docker run -d \
  --name rustdesk-starry-hbbs \
  --network host \
  --restart unless-stopped \
  -v /opt/rustdesk-server-starry/data:/root \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0 \
  hbbs --starry-config=/root/starry/config.yaml
```

单机部署使用同一持久目录启动官方 HBBR：

```sh
docker run -d \
  --name rustdesk-hbbr \
  --network host \
  --restart unless-stopped \
  -v /opt/rustdesk-server-starry/data:/root \
  rustdesk/rustdesk-server:1.1.16 \
  hbbr -k _
```

## 验证部署

Compose 静态检查只是第一层：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
```

查看当前 Relay 分配池：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"
```

使用两个真实公网出口地址预览规则结果：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 192.0.2.10 198.51.100.20 native\n' | nc -w 2 127.0.0.1 21115"
```

上面的地址是文档保留地址，必须替换为 HBBS 实际观察到的公网地址。`test-geo` 只预览
分配决定，不会建立会话，也不能证明两个客户端能连接 Relay。

最后根据实际部署使用真实客户端完成原生/P2P、原生 Relay、登录后的 Secure TCP、
WSS↔WSS 和两个方向的 WSS↔原生测试。详见
[`运维与完整验证`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。

## 升级与回滚

1. 在 `.env` 中记录当前运行标签或 digest。
2. 备份完整数据目录，尤其是 `id_ed25519`、`id_ed25519.pub`、数据库与 `starry/config.yaml`。
3. 阅读目标版本说明与配置迁移要求。
4. 拉取目标镜像并执行 `docker compose config --quiet`。
5. 重建服务、检查日志、执行管理命令并完成真实客户端会话。
6. 验收完成前保留旧镜像和备份。

从 patch-v1.1.0 回滚到 patch-v1.0.0 时，必须恢复 schema `version: 1` 配置。
patch-v1.0.0 不理解 schema v2；保留 v2 文档会使 Starry 配置被拒绝，HBBS 回退到
上游行为。

不要通过覆盖不可变版本标签来实施升级。

## 容器常见问题

| 现象 | 检查 |
| --- | --- |
| `config.yaml` 一直为空 | 首次启动为空是正常现象，请从生成的示例复制所需部分。 |
| 修改 YAML 后 Starry 功能消失 | 查看 HBBS 启动/重载错误；一个未知或无效字段会拒绝整份 Starry 配置。 |
| `test-geo` 返回 `""` | 检查 `relay_servers`、官方 HBBS Relay 在线状态、MMDB 可用性与规则内 Relay 名称。 |
| 原生正常但 WSS 失败 | 同时检查两条精确 Nginx 路径、证书域名、`trusted_proxies`、endpoint 覆盖与 `websocket-status`。 |
| API 登录成功但控制超时 | 将 API 登录与 HBBS Secure TCP 分层处理，检查 `21116/TCP`、HBBS 公钥和 `secure_tcp.mode`。 |
| 容器健康但桌面控制失败 | 进程和端口检查不是协议会话；对齐同一次连接的两端客户端与服务器日志。 |

按症状排查见
[`常见问题排查`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)。

## 许可证与声明

镜像按 AGPL-3.0 发布，由锁定的官方 RustDesk Server 源码加 Starry overlay
构建。这是非官方社区镜像，不包含 GeoLite2 数据库。部分代码和文档使用 AI 辅助
生成或修订，不附带任何额外保证。
