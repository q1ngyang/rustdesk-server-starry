# 快速开始：单机 Docker 完整部署

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Getting-Started) | **简体中文**

本教程从一台空白 Linux 服务器开始，在同一台主机上部署 Starry HBBS、镜像内附带的
HBBR，并依次启用安全 TCP（Secure TCP）、按地理位置选择中继服务器、TLS 和
WebSocket 信令。本教程不部署账户/API 服务。

以下命令以 `linux/amd64` 的 Debian 或 Ubuntu、具有 `sudo` 权限的用户，以及
`rustdesk.example.com` 这一个公网域名为例。实际操作时必须替换该域名。

## 1. 部署完成后的结构

```text
RustDesk 桌面客户端
  ├─ 21115/TCP        NAT 类型测试
  ├─ 21116/TCP+UDP    注册、信令、打洞和安全 TCP
  ├─ 21117/TCP        原生中继数据
  └─ 443/TCP          可选 WSS，由 Nginx 转发 /ws/id 和 /ws/relay

同机 Nginx
  ├─ /ws/id    -> 127.0.0.1:21118（HBBS）
  └─ /ws/relay -> 127.0.0.1:21119（HBBR）
```

HBBS 与 HBBR 容器使用同一个固定版本的 Starry 镜像。镜像内的 HBBR 没有加入 Starry
改动，而是从本版本锁定的同一份上游源码构建。这样可以避免另一个官方镜像单独更新，
造成 HBBS 与 HBBR 版本不一致。

## 2. 准备域名和服务器

为 `rustdesk.example.com` 创建指向服务器公网 IPv4 地址的 `A` 记录。只有服务器已经
正确配置且公网可访问 IPv6 时才添加 `AAAA` 记录。申请证书前先确认解析结果：

```sh
getent ahosts rustdesk.example.com
```

按照 [Docker 官方说明](https://docs.docker.com/engine/install/)安装 Docker Engine 和
Compose 插件，然后执行：

```sh
docker version
docker compose version
```

两条命令都成功后再继续。如果服务器已经运行其他 RustDesk Server，必须先备份身份密钥
并停止旧服务；同一组端口不能同时运行两套 HBBS。

## 3. 下载部署文件

```sh
sudo mkdir -p /opt/rustdesk-server-starry/data/starry
sudo chown -R "$(id -u):$(id -g)" /opt/rustdesk-server-starry
cd /opt/rustdesk-server-starry

curl -fsSLo compose.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/compose.yaml
curl -fsSLo .env \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/.env.example
curl -fsSLo data/starry/config.yaml \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/config/config.single-host.yaml
```

需要长期保存的内容如下：

| 主机路径 | 容器内路径 | 是否需要备份 |
| --- | --- | --- |
| `/opt/rustdesk-server-starry/data` | `/root` | **需要。** 包含身份密钥、SQLite 数据、Starry 配置和 MMDB 文件。 |
| `/opt/rustdesk-server-starry/compose.yaml` | 无 | 需要，用于记录服务编排。 |
| `/opt/rustdesk-server-starry/.env` | 无 | 需要，但应按私密文件保管；其中记录镜像版本和本机路径。 |
| `/etc/letsencrypt` | Nginx 主机目录 | **需要。** 由容器外的证书工具管理。 |

不得公开 `data/id_ed25519`、`.env` 或 TLS 私钥。

## 4. 检查必填和推荐设置

用编辑器打开 `.env`：

| 变量 | 要求 | 推荐值 |
| --- | --- | --- |
| `STARRY_IMAGE` | 必填 | `ghcr.io/q1ngyang/rustdesk-server-starry` |
| `STARRY_VERSION` | 必填 | 固定候选版本，当前为 `1.1.16-patch-v1.3.0`；生产环境不要使用 `latest`。 |
| `STARRY_DATA_DIR` | 必填 | 服务托管环境建议写成 `/opt/rustdesk-server-starry/data` 绝对路径。 |
| `RUSTDESK_LOG_LEVEL` | 推荐 | 保持 `info`；只在排查问题时临时改成 `debug`。 |
| 重启策略和容器名称 | 可选 | 没有本机冲突时保持示例值。 |

然后编辑 `data/starry/config.yaml`：

1. 把全部 `rustdesk.example.com` 替换成真实公网域名；
2. 保持 `secure_tcp.mode: auto`；
3. Nginx 和证书尚未正常工作前，保持 `websocket_signal.enabled: false`；
4. 合法来源的 MMDB 文件尚未就绪前，保持 `geo.enabled: false`；
5. 本教程不部署令牌签发服务，因此保持 `connection_auth.mode: off`。

该文件使用配置结构版本 `4`，这是 patch-v1.3.0 完整功能所需版本；Relay 质量仍需显式
开启。出现未知字段时，整份候选配置都会被拒绝，不要凭经验猜测配置项名称。

## 5. 配置防火墙

远程修改防火墙前，先放行服务器实际使用的 SSH 端口。RustDesk 需要以下规则：

| 端口 | 是否对公网开放 | 用途 |
| --- | --- | --- |
| `21115/TCP` | 是 | RustDesk NAT 类型测试。 |
| `21116/TCP` | 是 | 注册、信令和安全 TCP。 |
| `21116/UDP` | 是 | ID 注册和打洞。 |
| `21117/TCP` | 是 | HBBR 原生中继数据。 |
| `80/TCP` | 是 | 首次申请证书和跳转到 HTTPS。 |
| `443/TCP` | 启用 WSS 时开放 | `/ws/id` 和 `/ws/relay` 的公网 TLS 入口。 |
| `21118/TCP`、`21119/TCP` | **否** | 明文 WebSocket 后端，只允许本机 Nginx 访问。 |
| `21120/TCP` | **本教程不开放** | 可选管理代理端口，只应监听本机或私有管理网络。 |

UFW 示例（SSH 不是 `22` 时必须先修改第一条）：

```sh
sudo ufw allow 22/tcp
sudo ufw allow 21115/tcp
sudo ufw allow 21116/tcp
sudo ufw allow 21116/udp
sudo ufw allow 21117/tcp
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw deny 21118/tcp
sudo ufw deny 21119/tcp
sudo ufw enable
sudo ufw status numbered
```

云平台安全组也要设置相同的公网放行规则。修改主机防火墙不会自动修改云平台安全组。

## 6. 启动 HBBS 和 HBBR

```sh
cd /opt/rustdesk-server-starry
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 120 hbbs hbbr
```

第一条命令只检查编排文件，不能证明容器已经正常运行。继续检查运行文件：

```sh
test -s data/id_ed25519
test -s data/id_ed25519.pub
test -s data/starry/config.yaml
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}}'
docker inspect rustdesk-hbbr --format '{{.Config.Image}}'
```

两条 `docker inspect` 命令必须显示同一个 Starry 版本。HBBS 日志应明确显示 Starry 配置
已经加载；容器处于运行状态不代表候选配置已经生效。

## 7. 配置 RustDesk 客户端

在两台桌面客户端中打开“设置 → 网络”，填写：

| RustDesk 设置项 | 填写内容 |
| --- | --- |
| ID 服务器 | `rustdesk.example.com` |
| Key | `data/id_ed25519.pub` 的完整单行内容 |
| 中继服务器 | 留空，由 HBBS 按配置选择。 |
| API 服务器 | 本教程不部署 API，保持为空。 |
| 使用 WebSocket | 第一次测试先关闭。 |

只读取公钥，不要输出私钥：

```sh
cat /opt/rustdesk-server-starry/data/id_ed25519.pub
```

修改网络设置后，重新连接或重启两台客户端。

## 8. 验证原生连接和中继连接

按以下顺序验证：

1. 两台客户端都从当前服务器获得 ID；
2. 控制请求能够到达被控端；
3. 真实会话中键盘、鼠标和画面均正常；
4. 在必须经过中继的网络中测试；如果当前客户端提供“始终通过中继连接”选项，也可
   临时启用该选项；
5. 用两端客户端与 HBBR 日志核对同一次连接。

```sh
docker compose --env-file .env -f compose.yaml logs --since 10m hbbs hbbr
```

P2P 连接成功不能证明 HBBR 正常；端口能够建立 TCP 连接也不能证明 RustDesk 会话正常。

## 9. 放置 MMDB 并启用地理位置规则

镜像不包含 GeoLite2 数据库。请自行选择合法、可信的 MMDB 来源并遵守许可证。手动放置
Country 数据库的示例：

```sh
mkdir -p /opt/rustdesk-server-starry/data/mmdb
cp /path/to/GeoLite2-Country.mmdb \
  /opt/rustdesk-server-starry/data/mmdb/GeoLite2-Country.mmdb
```

主机的 `data/mmdb` 在 HBBS 容器内对应 `/root/mmdb`，与模板中的相对路径一致。
国家匹配需要 Country 数据库，或包含国家信息的 City 数据库；城市规则需要 City；
ASN 和运营商规则需要 ASN。

编辑 `data/starry/config.yaml`：

```yaml
geo:
  enabled: true
```

保留模板最后的全匹配兜底规则。如果已经取得授权的 MMDB HTTPS 文件直链，可填写对应
`mmdb.*.url`，将 `update_on_start` 改为 `true`，并保留每周更新间隔；否则保持 URL
为空，继续手动更换文件。

只重启 HBBS，然后检查配置是否接受以及是否存在数据库警告：

```sh
docker compose --env-file .env -f compose.yaml restart hbbs
docker compose --env-file .env -f compose.yaml logs --tail 150 hbbs
```

地理位置规则按优先级顺序执行，不是负载均衡：第一条匹配规则中的第一台可用中继服务器
会被选中。

## 10. 安装 Nginx 并申请证书

```sh
sudo apt update
sudo apt install -y nginx certbot python3-certbot-nginx
```

先下载临时 HTTP 配置，替换示例域名，启用站点并申请证书：

```sh
cd /opt/rustdesk-server-starry
curl -fsSLo nginx-bootstrap.conf \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/nginx/single-host.bootstrap.conf
sudo cp nginx-bootstrap.conf /etc/nginx/sites-available/rustdesk-starry.conf
sudo editor /etc/nginx/sites-available/rustdesk-starry.conf
sudo ln -sfn /etc/nginx/sites-available/rustdesk-starry.conf \
  /etc/nginx/sites-enabled/rustdesk-starry.conf
sudo nginx -t
sudo systemctl reload nginx
sudo certbot --nginx -d rustdesk.example.com
```

证书签发成功后下载最终配置，替换域名和证书路径，通过检查后再重新加载：

```sh
curl -fsSLo nginx-starry.conf \
  https://raw.githubusercontent.com/q1ngyang/rustdesk-server-starry/main/examples/nginx/single-host.example.conf
sudo cp nginx-starry.conf /etc/nginx/sites-available/rustdesk-starry.conf
sudo editor /etc/nginx/sites-available/rustdesk-starry.conf
sudo nginx -t
sudo systemctl reload nginx
```

如果 `nginx -t` 报错，应停止并修正具体错误。不要关闭证书校验，也不要把
`21118/21119` 直接暴露到公网。

## 11. 启用并验证 WebSocket 信令

编辑 `data/starry/config.yaml`：

```yaml
websocket_signal:
  enabled: true
```

模板已经设置为只信任本机 Nginx，并将当前中继服务器映射到
`wss://rustdesk.example.com/ws/relay`。桌面客户端通常不发送 `Origin` 请求头；没有部署
浏览器客户端时保持 `allowed_origins: []` 即可。

重启 HBBS，并至少等待一个配置的中继健康检查周期：

```sh
docker compose --env-file .env -f compose.yaml restart hbbs
docker compose --env-file .env -f compose.yaml logs --tail 180 hbbs
```

检查两条协议升级路径。看到 HTTP `101 Switching Protocols` 表示入口初步正常；由于升级
后的连接会保持打开，`curl` 最后可能显示超时。

```sh
for path in ws/id ws/relay; do
  curl --http1.1 --include --no-buffer --max-time 5 \
    -H 'Connection: Upgrade' \
    -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Version: 13' \
    -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    "https://rustdesk.example.com/$path"
done
```

在两台客户端上启用“使用 WebSocket”，完成一次真实远程控制；需要混合接入时，再测试
一端使用 WebSocket、另一端保持原生连接。仅有 HTTP `101` 不能作为最终验收结果。

## 12. 备份并记录部署

需要备份：

```text
/opt/rustdesk-server-starry/data/
/opt/rustdesk-server-starry/.env
/opt/rustdesk-server-starry/compose.yaml
/etc/letsencrypt/
/etc/nginx/sites-available/rustdesk-starry.conf
```

其中至少要保护 `data/id_ed25519`、`data/id_ed25519.pub`、
`data/db_v2.sqlite3`、`data/starry/config.yaml` 和 `data/mmdb/`。只有实际演练过恢复，
才能确认备份可用。

## 13. 本教程有意不启用的功能

连接认证保持 `off`，因为 JWT 的签发和撤销需要兼容的账户/API 服务。Starry 可以搭配
第三方 API；同一开发者维护的推荐项目是
[rustdesk-api-kessoku](https://github.com/q1ngyang/rustdesk-api-kessoku)。添加 API 前请先阅读
[Kessoku Wiki](https://github.com/q1ngyang/rustdesk-api-kessoku/wiki)和
[账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration)。
Kessoku + Starry 联合部署专页仍在编写中。

可选的管理代理（Control Agent）不是 RustDesk 数据传输的必需组件。只有需要经过认证的
中继状态查询或受控配置变更时才部署，并从只读模式开始。

继续阅读：

- [Docker 部署参考](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)
- [配置参数详解](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Configuration-Reference)
- [客户端配置](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Client-Configuration)
- [运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
- [常见问题排查](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Troubleshooting)
