# 快速开始

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) | **简体中文**

本指南从一台空白 Linux 服务器开始，先完成经过验证的原生 RustDesk 连接。只有基础
链路正常后，才逐项增加 Geo 规则、API 登录和 WebSocket。

## 1. 选择部署方式

| 方式 | 适用场景 | 说明 |
| --- | --- | --- |
| Docker Compose | 大多数 Linux 服务器 | 推荐，最容易复现、备份和回滚。 |
| DEB 包 | 使用原生 systemd 的 Debian/Ubuntu | HBBS、HBBR 和工具是独立包。 |
| 独立二进制 | 自行管理路径与服务 | 适用于其他 Linux 发行版和 Windows。 |
| 从源码应用 overlay | 开发或审计 patch | 不是最短生产路径。 |

本页后续使用 Docker Compose。

## 2. 准备服务器

需要：

- 支持的 `linux/amd64` 或 `linux/arm64` 主机；
- Docker Engine 与 Compose 插件；
- 可持久保存密钥、日志、SQLite 与 MMDB 的目录；
- 两端客户端可访问的公网 IP 或 DNS；
- 主机防火墙控制权；
- 替换现有 RustDesk Server 前的有效备份。

基础原生端口：

| 端口 | 第一次原生测试是否需要？ |
| --- | --- |
| `21115/TCP` | 标准 NAT 测试需要，但不要通过 HTTP 代理公开 Starry 管理入口。 |
| `21116/TCP` | 需要。 |
| `21116/UDP` | 需要。 |
| `21117/TCP` | 会话需要 Relay 时需要。 |
| `21118/TCP`、`21119/TCP`、`443/TCP` | 启用 WebSocket 前不需要。 |

## 3. 下载示例

```sh
sudo mkdir -p /opt/rustdesk-server-starry
sudo chown "$(id -u):$(id -g)" /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLO \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.yaml
curl -fsSLo .env \
  https://github.com/q1ngyang/rustdesk-server-starry/releases/latest/download/compose.env.example
mkdir -p data
```

检查 `.env`。生产使用不可变标签；官方 HBBR 版本应与 Starry 完整版本中的官方前缀匹配。

## 4. 校验并启动

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
```

静态校验失败时立即停止并修正错误。不要在不了解原因时换用其他 Compose 或删除必填值。

检查启动证据：

```sh
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbs hbbr
test -s data/id_ed25519
test -s data/id_ed25519.pub
test -f data/starry/config.yaml
test -f data/starry/config.example.yaml
```

首次启动时真正生效的 `data/starry/config.yaml` 为空，这是正常设计。

## 5. 首先只启用 Secure TCP

复制最小配置：

```sh
cp \
  /path/to/repository/config/config.minimal.yaml \
  data/starry/config.yaml
```

或写入等价内容：

```yaml
version: 1

secure_tcp:
  mode: auto
```

热加载并读取结果：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
```

一个无效或未知字段会拒绝整份 Starry 文档。应修正报告的字段，不要假设部分配置已经生效。

## 6. 配置两端客户端

在两端打开**设置 → 网络**并填写：

| 字段 | 第一次测试值 |
| --- | --- |
| ID Server | HBBS 公网 DNS 或 IP。 |
| Key | `data/id_ed25519.pub` 的完整单行内容。 |
| Relay Server | 保持为空，由 HBBS 动态分配。 |
| API Server | 第一次原生测试保持为空。 |
| 使用 WebSocket | 关闭。 |

若当前客户端版本在修改网络设置后没有立即重新注册，请重启或重新连接客户端。

## 7. 验证原生链路

按以下顺序测试：

1. 两端客户端都能从自建服务获得 ID；
2. 控制请求能到达远端；
3. 桌面会话能双向传输画面和输入；
4. 如果当前是 P2P，应在适当网络条件下另外测试 Relay，不能据此假定 HBBR 正常；
5. 任一层失败时，收集同一次连接的日志。

公钥必须与中心 HBBS 一致。API 凭据、Relay 公钥字段或控制 session 标识都不能替代该值。

## 8. 增加功能前备份

至少备份：

```text
data/id_ed25519
data/id_ed25519.pub
data/db_v2.sqlite3（存在时）
data/starry/config.yaml
```

私钥必须保密。丢失私钥会改变客户端信任的公钥；泄露私钥则必须轮换服务器身份。

## 9. 一次只增加一类功能

推荐顺序：

1. [GEO 规则入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics)
2. 需要时进行[多节点部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment)
3. 需要账户功能时独立选择 API
4. [反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)
5. schema v2 与可选 WebSocket Signal
6. [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)

每次变更后保留最后可用配置，并完成该层对应的检查。
