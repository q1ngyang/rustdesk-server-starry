# 版本升级与回滚

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) | **简体中文**

升级同时涉及两个版本：官方 RustDesk Server 基线和 Starry patch。例如
`1.1.16-patch-v1.1.0` 表示官方服务端 `1.1.16` 加 Starry patch `1.1.0`。生产环境
应锁定完整标签，并记录实际镜像摘要。

## 升级原则

- 同时阅读 Starry 发布说明和上游 RustDesk Server 变更；
- 拉取或替换前备份持久化数据目录；
- 保留 `id_ed25519`，更换它会改变服务端身份；
- 验收完成前保留旧镜像标签、二进制/包、配置和已验证 Compose；
- 先更换二进制/镜像，再启用新 schema 或传输功能；
- 每次只升级一个中心；不能让重复 HBBS 共用公网端口和数据目录；
- 未测试路径必须标为未测试，静态检查不等于运行时验收。

## 当前 patch 说明

- [patch-v1.1.0 中文发布说明](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/RELEASE-NOTES-patch-v1.1.0.zh-CN.md)
- [中文更新日志](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CHANGELOG.zh-CN.md)

patch v1.1.0 增加可选的持久 `/ws/id` 信令、WSS/原生混合会话、经证书校验的 WSS
Relay 健康、schema v2 限制和传输感知选择。schema v1 继续有效，并保持 WebSocket
Signal 关闭。

## 1. 盘点和备份

在部署目录执行：

```sh
set -eu
date -u
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml config --images
docker inspect rustdesk-starry-hbbs --format '{{.Config.Image}} {{json .Image}}'

backup_dir="../rustdesk-starry-backup-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$backup_dir"
cp -a data "$backup_dir/data"
cp -a .env compose.yaml "$backup_dir/"
sha256sum "$backup_dir/data/id_ed25519" \
  "$backup_dir/data/id_ed25519.pub" \
  "$backup_dir/data/starry/config.yaml"
```

备份包含服务端私钥，也可能包含数据库或账户相关状态，必须妥善保护。确认文件非空，
并且备份位于活动绑定目录之外。

可选第三方 API 应另外遵循其数据库一致性备份流程。Starry 无法保证其他项目的状态
格式或迁移行为。

## 2. 准备候选配置

从 patch v1.0.0 升级 v1.1.0 时，第一次替换镜像/二进制仍使用原 schema v1。另存
一份 schema v2 候选：

```yaml
version: 2

# 原有 relay_servers、secure_tcp、mmdb 和 geo 部分保留在这里。

websocket_signal:
  enabled: false
  # 在这里定义限制、可信代理和精确 Relay 健康 endpoint。
```

启用 WebSocket 前，endpoint Relay 名称必须恰好覆盖 `relay_servers`。此时不要覆盖
活动配置。

## 3. 只拉取和检查，不启动

在 `.env` 设置新的不可变版本：

```dotenv
STARRY_VERSION=1.1.16-patch-v1.1.0
RUSTDESK_SERVER_VERSION=1.1.16
```

然后执行：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml pull
docker compose --env-file .env -f compose.yaml images
```

若策略锁定摘要，应将拉取摘要与已审核的 Release/Package 值比较。不能根据可变
`latest` 标签判断镜像身份。

## 4. 只替换目标服务

```sh
docker compose --env-file .env -f compose.yaml up -d hbbs hbbr
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr
```

继续使用旧 schema v1，验证：

- 两个服务持续稳定并使用原密钥；
- 原生注册正常；
- 部署 API 时，登录后的原生 Secure TCP 正常；
- P2P 和原生 Relay 正常；
- 原生 Geo 与故障切换结果保持预期。

原生基线出现回归，应立即停止。

## 5. 先引入关闭 WebSocket 的 schema v2

将候选文件安装为 `data/starry/config.yaml`，再重载：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"
docker logs --tail 200 rustdesk-starry-hbbs
```

响应和日志必须确认整份配置被接受，再次验证原生会话。HBBS 没有退出并不代表 schema
变更成功；无效 Starry 配置会有意恢复上游行为。

## 6. 分阶段部署 TLS 并启用 WebSocket

1. 部署精确 Nginx `/ws/id` 和 `/ws/relay` location；
2. 执行 `nginx -t`、重载 Nginx，并校验公网证书域名；
3. 验证每个 endpoint 的 HTTP Upgrade；
4. 设置 `websocket_signal.enabled: true` 并重载 Starry；
5. 等待一个健康间隔并查看 `websocket-status`；
6. 对 `native`、`wss` 和 `mixed` 执行 `test-geo`；
7. 用真实客户端测试 WSS 到 WSS 和两个混合方向；
8. 扩大范围前测试 Relay 故障与恢复。

使用[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
中的完整清单。

## 功能回滚

如果只发生 WebSocket Signal 回归，原生行为仍正常：

1. 设置 `websocket_signal.enabled: false`；
2. 执行 `reload-starry-config`；
3. 确认管理响应和 HBBS drain 日志；
4. 关闭客户端 WebSocket 或撤销下发策略；
5. 重新验证原生注册、P2P、Secure TCP 和原生 Relay。

根据回滚策略关闭或停止使用公网 Nginx 路由。关闭功能后，已有 WSS session 会被 drain。

## 镜像回滚

从已审核备份恢复 `.env` 和配置：

```sh
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/.env .env
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/compose.yaml compose.yaml
cp -a ../rustdesk-starry-backup-YYYYMMDDTHHMMSSZ/data/starry/config.yaml \
  data/starry/config.yaml

docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d hbbs hbbr
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr
```

替换占位备份路径前，先验证其解析后的实际位置和内容。除非数据格式兼容性确实要求，
且服务已经停止，否则不要把整份旧数据目录覆盖到活动部署。通常先保留当前密钥/状态，
只恢复兼容配置和旧镜像更安全。

回滚后重复原生及适用的 API/Relay 验收。

## DEB 或独立二进制升级

DEB 部署应下载匹配架构的包、校验 Release 摘要、备份
`/var/lib/rustdesk-server-starry`，再用包管理器安装。逐个服务重启检查：

```sh
sudo systemctl restart rustdesk-server-starry-hbbs
sudo systemctl status rustdesk-server-starry-hbbs --no-pager
sudo journalctl -u rustdesk-server-starry-hbbs -n 200 --no-pager

sudo systemctl restart rustdesk-server-starry-hbbr
sudo systemctl status rustdesk-server-starry-hbbr --no-pager
```

HBBR 包是从固定官方 revision 构建的未修改上游 HBBR。回滚需要保留旧包或仓库快照，
不能假定包缓存仍然保存它们。

独立二进制应使用带版本的文件名，以原子方式更新服务软链接或路径。记录摘要并备份前，
绝不能覆盖唯一一份已知可用二进制。

## 出现以下情况应停止并回滚

- Starry 配置被拒绝或意外恢复上游行为；
- 现有密钥改变或持久化文件消失；
- 原生注册或原生 Relay 回归；
- 原先可用的已认证客户端 Secure TCP 失败；
- 所需传输没有符合条件的 Relay；
- 超过文档阈值后 WSS 健康仍未 Ready；
- 两台客户端无法完成所需控制/数据测试。

发布检查和 CI 是有价值的 Release 证据，但无法替代在实际 DNS、证书、代理、网络和
客户端上的验收。
