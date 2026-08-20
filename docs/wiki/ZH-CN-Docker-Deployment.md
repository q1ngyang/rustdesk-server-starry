# Docker 部署

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Deployment) | **简体中文**

Linux 上的 Docker Compose 是推荐部署方式。本指南解释完整单机示例，不把
`compose up` 当成服务已经可用的证明。

## 架构

```text
RustDesk 客户端
    │
    ├── 21116 TCP/UDP ──> Starry HBBS
    │                        │
    │                        └── 选择 HBBR
    │
    └── 21117 TCP ─────> 官方 HBBR

共享持久目录：身份、数据库、日志、Starry 配置、MMDB
```

示例使用 host 网络在一台 Linux 主机运行两个服务。HBBR 是未修改的官方组件。

## 文件

- [`examples/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [`examples/.env.example`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/.env.example)
- [`config/config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml)
- [`examples/nginx/center.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/center.example.conf)，仅在使用 HBBS WSS 时需要
- [`examples/nginx/api.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/api.example.conf)，仅在使用可选 API 时需要

## 准备目录

```sh
sudo install -d -m 0750 -o "$(id -u)" -g "$(id -g)" \
  /opt/rustdesk-server-starry/data
cd /opt/rustdesk-server-starry

cp /path/to/repository/examples/compose.yaml .
cp /path/to/repository/examples/.env.example .env
```

检查 `.env` 中的精确镜像。示例将 `1.1.16-patch-v1.2.0` 与官方 HBBR
`1.1.16` 对齐。

## 静态校验

```sh
docker compose --env-file .env -f compose.yaml config
docker compose --env-file .env -f compose.yaml config --quiet
```

第一条命令用于查看合并后的配置。确认：

- 镜像标签符合预期；
- 数据目录是预期的绝对或相对路径；
- 只有一份 HBBS 和一份 HBBR；
- 使用 host 网络；
- 没有注入真实秘密或非预期环境变量。

## 首次启动与身份

```sh
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
```

确认 `data/id_ed25519` 与 `data/id_ed25519.pub` 非空。私钥只保留在中心，并在
客户端推广前建立受保护备份。

HBBS healthcheck 只验证本机进程可达和密钥文件，不验证外部路由或桌面会话。

## 配置 Starry

第一次连接先把最小模板复制到实际生效路径：

```sh
cp /path/to/repository/config/config.minimal.yaml \
  data/starry/config.yaml

docker exec rustdesk-starry-hbbs sh -c \
  "test -s /starry/config.yaml"
docker compose --env-file .env -f compose.yaml restart hbbs
```

之后把 Geo 与 WebSocket 分成两次变更。参见
[配置参数参考](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)。

## 防火墙

原生 RustDesk 至少允许入站 `21116/TCP`、`21116/UDP`、`21117/TCP`。
`21115/TCP` NAT 测试按官方 RustDesk 指引配置，但不要为回环管理命令创建公网 HTTP 入口。

启用 WSS 时：

- 通过 Nginx 开放 `443/TCP`；
- `21118/TCP`、`21119/TCP` 只允许本机或明确可信的代理网络访问；
- 如果客户端可能关闭 WebSocket，应保留原生端口。

## 客户端设置

两端配置中心 ID Server 和 `data/id_ed25519.pub` 的完整值。Relay Server 保持为空；
第一次原生测试时 API 与 WebSocket 都关闭。

参见[客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration)。

## 验证

必须完成以下各项，不能只看 `docker compose ps`：

1. HBBS/HBBR 日志没有启动错误；
2. 两端客户端都注册到自建 ID Server；
3. 原生控制请求和桌面会话正常；
4. 验收范围包含 HBBR 时，实际建立一次 Relay 会话；
5. 部署 API 后验证登录状态下的 Secure TCP；
6. 先预览 Geo 决策，再在真实 Relay 会话中验证；
7. 启用 WSS 时验证 WSS↔WSS 和两个方向的混合会话。

命令与证据要求见
[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。

## 备份与升级

修改镜像前备份整个数据目录。记录当前标签/digest，阅读上游与 Starry 版本说明，
拉取目标、执行静态校验、重建并重复真实客户端验收。验证完成前保留旧镜像和配置。

参见[版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)。
