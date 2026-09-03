# rustdesk-server-starry 容器镜像

[English](CONTAINER.md) | **简体中文**

本文面向从 GHCR 包页面进入、需要直接部署镜像的用户。项目介绍和源码关系见主
[`README.zh-CN.md`](../project/README.zh-CN.md)。

部署入口：

- [GHCR 镜像页面](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)
- [零基础单机部署教程](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)
- [推荐 Docker 部署指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)
- [单机编排示例](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [管理代理编排示例](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/control-agent/compose.yaml)

## 镜像包含什么

patch-v1.3.1 开发镜像以 `linux/amd64` 为正式构建和运行验收平台，由一个锁定的官方
RustDesk Server 版本加 Starry HBBS 扩展层构建。ARM 仅尽力保持源码兼容，不属于
v1.3.1 承诺的镜像平台。

| 命令 | 来源 | 用途 |
| --- | --- | --- |
| `hbbs` | 官方 HBBS 加 Starry 扩展层 | ID 注册、会合、信令、安全 TCP、按地理位置选择中继服务器和可选 WebSocket 信令。 |
| `hbbr` | 上游中继数据路径 + 公开探测/认证遥测 | 与 HBBS 从同一锁定上游版本构建，响应不含负载明细的 Akari 质量探测；有界负载/版本遥测仅供 HBBS 认证拉取；所有示例都使用同一镜像版本。 |
| `rustdesk-utils` | 未修改的上游工具 | 密钥和数据库维护工具。 |
| `starry-control-agent` | Starry 可选 Linux 管理组件 | 管理一台本机 HBBS 的固定接口；强制使用 mTLS 与按权限划分的服务令牌，默认禁止写入配置。 |
| `starry-relayctl` | Starry Relay enrollment 工具 | 本地生成节点密钥/CSR，并把一次 SP1 结果安装到 `RELAY_DATA_DIR`；不修改兼容上游的 `hbbr` CLI。 |

镜像**不包含**账户/API 服务，也不包含任何 GeoLite2/MMDB 数据库；管理代理不是账户
API。可以另行部署兼容的第三方 API，推荐
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku)。
请审查各组件许可证，并将秘密保留在镜像之外。

## 选择标签

发布标签格式为：

```text
<官方-rustdesk-server-版本>-patch-vX.Y.Z
```

例如：

```text
1.1.16-patch-v1.3.1
```

- 日常生产部署使用不可变版本标签。
- 必须完全锁定镜像内容时使用镜像摘要。
- `latest` 跟随最近一次成功发布的 Starry 版本，仅适合评估，或已建立自动更新与
  回滚流程的运维者。

拉取当前文档对应版本：

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1
```

公开 GHCR 镜像可匿名拉取。上线前检查实际镜像摘要和平台：

```sh
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1 \
  --format '{{json .RepoDigests}}'

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1
```

## 推荐快速部署

仓库的 [`examples/compose.yaml`](../../examples/compose.yaml) 会启动：

- 本镜像中的 Starry HBBS；
- **同一个固定版本 Starry 镜像**中保留上游字节转发路径、增加公开质量探测和认证负载遥测的 HBBR。

在 Linux Docker 主机上执行：

```sh
mkdir -p /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLo compose.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/compose.yaml
curl -fsSLo .env \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/.env.example

mkdir -p data/starry
curl -fsSLo data/starry/config.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/config/config.single-host.yaml
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

示例要求宿主机使用一个绝对 `STARRY_PERSIST_ROOT`，并把 HBBS、Control 状态、配置与
Relay 状态分别挂载。HBBS 仍在 `/root` 看到自己的子目录；HBBR 通过
`RELAY_DATA_DIR` 在 `/var/lib/rustdesk-server-starry/relay` 看到独立目录。配对与
enrollment 会拒绝 overlay/tmpfs、危险权限和根目录外身份路径；镜像不再声明匿名
`VOLUME` 兜底。

Starry HBBS 首次启动会创建：

```text
persist/hbbs/
├── id_ed25519
├── id_ed25519.pub
└── starry/
    ├── config.yaml
    └── config.example.yaml
```

`starry/config.yaml` 初始为空，这是有意设计。空配置或无效配置会禁用整份 Starry
配置，使 HBBS 使用上游命令行行为。请从 `starry/config.example.yaml` 复制需要的
部分，或从 [`config/`](../../config) 中选择入门模板。

绝不要把 `id_ed25519` 复制到纯 Relay 节点或文档中。公钥 `id_ed25519.pub` 是客户端
需要配置的 Key，可以分发。

## 端口

| 端口 | 协议 | 进程 | 用途 |
| --- | --- | --- | --- |
| `21115` | TCP | HBBS | NAT 测试和仅限回环地址的管理命令；不要公开代理管理入口。 |
| `21116` | TCP | HBBS | 原生信令、打洞协调与安全 TCP。 |
| `21116` | UDP | HBBS | 注册与心跳。 |
| `21117` | TCP | HBBR | 原生中继数据。 |
| `21118` | TCP | HBBS | `/ws/id` 明文 WebSocket 后端；只允许可信反向代理访问。 |
| `21119` | TCP | HBBR | `/ws/relay` 明文 WebSocket 后端；只允许可信反向代理访问。 |
| 配置值，通常 `21119` | UDP | HBBR | 可选 AKR1 FastMedia 数据面；仅在 schema v5 显式开启且认证遥测报告健康时开放。 |
| `21120` | TCP | 可选管理代理 | 私有 mTLS 管理接口；普通编排示例不对外发布该端口，只允许本机或私有管理网络访问。 |
| `443` | TCP | Nginx | 使用 WebSocket 或 HTTPS API 时的公网 TLS/WSS 入口。 |

只开放部署所需入口。WebSocket 客户端必须同时具备证书有效的 `/ws/id` 和
`/ws/relay`；只发布其中一条路径不构成完整 WebSocket 部署。

## 配置 Starry 功能

编辑主机上生成的文件：

```sh
vi data/starry/config.yaml
```

首次配置建议重启 HBBS；后续受管理的变更使用管理代理先预览再应用，或执行运行时重新加载：

```sh
docker restart rustdesk-starry-hbbs
```

如果重新加载返回错误，整份待加载配置都不会生效，HBBS 会保留最近一次有效配置。若
从未加载过有效配置，HBBS 保持上游兼容行为。恢复或修正文件后应再次加载，并确认配置
代次、内容摘要和各子系统结果一致；进程仍在运行不代表新配置已经启用。

可从以下模板开始：

- [`config/config.single-host.yaml`](../../config/config.single-host.yaml)：单机完整接入配置；
- [`config/config.minimal.yaml`](../../config/config.minimal.yaml)：仅启用安全 TCP；
- [`config/config.geo-basic.yaml`](../../config/config.geo-basic.yaml)：按国家依次选择中继服务器；
- [`config/config.geo-advanced.yaml`](../../config/config.geo-advanced.yaml)：嵌套城市/ASN/ISP 规则；
- [`config/config.websocket.yaml`](../../config/config.websocket.yaml)：配置结构版本 2 的 WebSocket 信令和中继健康检查；
- [`config/config.auth-audit.yaml`](../../config/config.auth-audit.yaml)：配置结构版本 3 的连接认证审计示例。

可选管理代理使用独立的 [`Compose 示例`](../../examples/control-agent/compose.yaml)与
[`运维指南`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)。
请先按只读模式接入，绝不能通过公网 RustDesk 端口开放管理代理或 HBBS 本地管理通道。

启用 Relay 质量前，应把 `RELAY_MAX_SESSIONS`（传给 HBBR 的
`STARRY_RELAY_MAX_SESSIONS`）设为真实并发会话容量。HBBR 会将该使用率与当前汇总带宽
使用率取较高值；`TOTAL_BANDWIDTH` 仍以 Mbit/s 表示。

## 不使用 Compose 执行命令

长期服务推荐 Compose。临时检查可执行：

```sh
docker run --rm \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1 \
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
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1 \
  hbbs --starry-config=/root/starry/config.yaml
```

单机部署从同一固定版本的 Starry 镜像启动 HBBR，但使用独立 Relay 持久目录：

```sh
docker run -d \
  --name rustdesk-hbbr \
  --network host \
  --restart unless-stopped \
  -e RELAY_DATA_DIR=/var/lib/rustdesk-server-starry/relay \
  -e STARRY_REQUIRE_PERSISTENT_STATE=1 \
  -v /opt/rustdesk-server-starry/relay:/var/lib/rustdesk-server-starry/relay \
  -v /etc/machine-id:/etc/machine-id:ro \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.3.1 \
  starry-relay-entrypoint -k _
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
  "echo '请改用已认证的 Control Agent GET /control/v1/relays'"
```

使用两个真实公网出口地址预览规则结果：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "echo '请改用已认证的 POST /control/v1/allocations:simulate'"
```

上面的地址是文档保留地址，必须替换为 HBBS 实际观察到的公网地址。`test-geo` 只预览
分配决定，不会建立会话，也不能证明两个客户端能连接 Relay。

最后根据实际部署使用真实客户端完成原生/P2P、原生 Relay、登录后的 Secure TCP、
WSS↔WSS 和两个方向的 WSS↔原生测试。详见
[`运维与完整验证`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。

## 升级与回滚

1. 在 `.env` 中记录当前运行标签或镜像摘要。
2. 备份完整数据目录，尤其是 `id_ed25519`、`id_ed25519.pub`、数据库与 `starry/config.yaml`。
3. 阅读目标版本说明与配置迁移要求。
4. 拉取目标镜像并执行 `docker compose config --quiet`。
5. 重建服务、检查日志、执行管理命令并完成真实客户端会话。
6. 验收完成前保留旧镜像和备份。

patch-v1.3.1 回滚 patch-v1.3.0 前，须关闭 FastMedia、取得匹配 apply ACK、等待
授权/allocation/stream drain，确认每张配对证书剩余至少九十天，并用
`starry-control-agent config downgrade --to-schema 4 --output ...` 导出 schema v4。
保留全部 pairing/enrollment 文件；v1.3.0 只忽略这些新状态，并通过不含 secret 值的
`relay-compat.env` 继续普通 Relay 与遥测。

从 patch-v1.3.0 回滚到 patch-v1.2.0 前，必须先恢复不含 `fast_mode` 和
`relay_quality` 的配置结构 `version: 3`（或更早）；二进制回滚前应关闭 FastCompat 并
等待超过授权 TTL。之后若从 patch-v1.2.0 回滚 patch-v1.1.0，须恢复 `version: 2`
（或更早）；patch-v1.1.0 不支持 `version: 3`。回滚到更早版本时也必须恢复该版本支持的
配置结构，不能依赖校验失败后的兼容行为。

不要通过覆盖不可变版本标签来实施升级。

## 容器常见问题

| 现象 | 检查 |
| --- | --- |
| `config.yaml` 一直为空 | 首次启动为空是正常现象，请从生成的示例复制所需部分。 |
| 修改 YAML 后 Starry 功能消失 | 查看 HBBS 启动/重载错误；一个未知或无效字段会拒绝整份 Starry 配置。 |
| `test-geo` 返回 `""` | 检查 `relay_servers`、HBBS 记录的中继在线状态、MMDB 可用性和规则内中继名称。 |
| 原生连接正常但 WSS 失败 | 同时检查两条 Nginx 精确路径、证书域名、`trusted_proxies`、健康检查地址覆盖和 `websocket-status`。 |
| API 登录成功但控制超时 | 分别排查 API 登录和 HBBS 安全 TCP，检查 `21116/TCP`、HBBS 公钥和 `secure_tcp.mode`。 |
| 容器健康但桌面控制失败 | 进程和端口检查不是协议会话；对齐同一次连接的两端客户端与服务器日志。 |

按症状排查见
[`常见问题排查`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)。

## 许可证与声明

镜像按 AGPL-3.0 发布，由锁定的官方 RustDesk Server 源码加 Starry 扩展层
构建。这是非官方社区镜像，不包含 GeoLite2 数据库。部分代码和文档使用 AI 辅助
生成或修订，不附带任何额外保证。
