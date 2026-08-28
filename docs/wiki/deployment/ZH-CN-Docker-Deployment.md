# Docker 部署参考

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) | **简体中文**

推荐在 `linux/amd64` 主机上使用 Docker Compose 部署 Starry。如果这是你第一次自建
RustDesk 服务器，请先按[单机完整教程](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)
逐步操作。本页用于后续查阅编排文件、持久化目录、网络、配置和变更流程。

## 项目提供的编排文件

| 场景 | 编排文件和环境变量 | 用途 |
| --- | --- | --- |
| 单机 | `examples/compose.yaml`、`examples/.env.example` | 一台服务器运行 Starry HBBS 和 HBBR；推荐从这里开始 |
| 中心节点 | `examples/center/compose.yaml`、`examples/center/.env.example` | 一台 HBBS 中心、同机 HBBR 和多台远程中继节点 |
| 纯中继节点 | `examples/relay/compose.yaml`、`examples/relay/.env.example` | 一台远程 HBBR |
| 管理代理 | `examples/control-agent/compose.yaml`、`examples/control-agent/.env.example` | HBBS、HBBR 和可选的私有管理代理 |

所有示例中的 HBBR 都与 HBBS 使用**同一个固定版本的 Starry 镜像**。HBBR 是从该版本
锁定的 RustDesk Server 上游源码构建的未经修改副本。不要在这些示例中换用会独立更新
的官方镜像。

## 单机服务结构

```text
公网
  ├─ 21115/TCP ───────────────> HBBS NAT 类型测试
  ├─ 21116/TCP+UDP ───────────> Starry HBBS
  ├─ 21117/TCP ───────────────> 镜像内 HBBR（上游中继数据路径）
  └─ 443/TCP ──> Nginx
                   ├─ /ws/id ─────> 127.0.0.1:21118（HBBS）
                   └─ /ws/relay ──> 127.0.0.1:21119（HBBR）

主机数据目录 ────────────────> 两个容器内的 /root
```

示例使用主机网络，以保留地理位置规则需要的客户端源地址，并让 RustDesk 端口直接监听
Linux 主机。它们不适用于 Docker Desktop。

## 持久化目录

HBBS 与 HBBR 都把同一个主机持久化目录挂载到 `/root`。升级前应整体备份该目录。

| `/root` 下的路径 | 含义 | 操作要求 |
| --- | --- | --- |
| `id_ed25519` | 服务器身份私钥 | 严格保密；不得复制到客户端或纯中继节点 |
| `id_ed25519.pub` | 服务器公钥 | 备份；把单行内容填写到客户端和纯中继节点 |
| `db_v2.sqlite3` | RustDesk 服务器数据 | 做一致性备份 |
| `starry/config.yaml` | Starry 待加载配置 | 私下保留版本记录，每次修改后验证 |
| `starry/config.example.yaml` | 首次启动生成的本地参考 | 仅供参考，不能认为它已经生效 |
| `mmdb/*.mmdb` | 部署者提供的地理位置数据库 | 定期更新并遵守数据许可证 |

替换容器时不得意外生成新的服务器身份。如果 `id_ed25519` 丢失，应先停止服务并恢复
数据目录，不要立即让所有客户端改用新公钥。

## 必填和推荐设置

`.env` 中的主要设置：

| 设置 | 要求 |
| --- | --- |
| `STARRY_IMAGE` | 必填；除非使用经过验证的私有镜像仓库，否则保持 GHCR 地址 |
| `STARRY_VERSION` | 必填；生产环境固定具体发布版本，不使用 `latest` |
| `STARRY_DATA_DIR` 或 `STARRY_PERSIST_ROOT` | 必填；使用专用、可备份的主机绝对路径 |
| `RUSTDESK_LOG_LEVEL` | 建议保持 `info`，只在排查期间临时使用 `debug` |

完整单机部署建议从
[`config/config.single-host.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.single-host.yaml)
开始。替换示例域名，并保留以下初始设置：

- `secure_tcp.mode: auto`；
- 所需 MMDB 文件准备完成前保持 `geo.enabled: false`；
- TLS 和两条代理路径验证通过前保持 `websocket_signal.enabled: false`；
- 部署兼容的令牌签发服务并完成审计前，保持 `connection_auth.mode: off`。

配置中出现未知字段或依赖不满足时，整份待加载配置都会被拒绝。每次重启或通过管理接口
启用配置后，都要检查 HBBS 日志。

## 启动或更新服务

```sh
docker compose --env-file .env -f compose.yaml config
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 120 hbbs hbbr
```

确认 HBBS 与 HBBR 最终使用同一镜像：

```sh
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}}'
docker inspect rustdesk-hbbr --format '{{.Config.Image}}'
```

编排文件检查通过、容器显示运行都只是局部结果。必须再完成真实原生会话和真实中继会话，
才能确认部署可用。

## 防火墙和反向代理

原生连接需要对公网开放 `21115/TCP`、`21116/TCP`、`21116/UDP`、`21117/TCP`。
申请证书和 HTTP 跳转需要 `80/TCP`；启用 WSS 时开放 `443/TCP`。不得把
`21118/TCP`、`21119/TCP` 暴露到公网，它们只允许本机可信 Nginx 访问。可选管理代理的
`21120/TCP` 只应监听本机或私有管理网络。

单机 Nginx 完整示例：

- `examples/nginx/single-host.bootstrap.conf`：首次申请证书时使用；
- `examples/nginx/single-host.example.conf`：证书就绪后代理 `/ws/id` 和 `/ws/relay`。

中心节点和独立中继节点的配置及验证方法见
[反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)。

## 账户与 API 服务

镜像和编排示例都不包含账户/API 服务。可以另行部署兼容的第三方 API；推荐使用
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku)。
接入前请阅读[账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration)。
Kessoku 联合部署 Wiki 页面完成后，会在该页补充链接。

## 变更和回退流程

1. 备份数据目录、`.env`、编排文件、Starry 配置和 TLS 文件。
2. 镜像、Starry 配置、反向代理或 API 每次只改一层。
3. 重建容器前先检查编排文件。
4. 检查 HBBS/HBBR 日志，并确认新配置已经被接受。
5. 对所有已启用路径重新验证原生连接、强制中继、地理位置规则和 WSS。
6. 验收失败时恢复上一版固定镜像和配置，不要删除持久化数据。

完整验收和恢复清单见
[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
与[版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)。
