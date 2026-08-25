# 版本升级与回滚

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) | **简体中文**

升级同时涉及两个版本：官方 RustDesk Server 基线和 Starry patch。例如
`1.1.16-patch-v1.2.0` 表示官方服务端 `1.1.16` 加 Starry patch `1.2.0`。生产环境
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

- [patch-v1.2.0 中文发布说明](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/RELEASE-NOTES-patch-v1.2.0.zh-CN.md)
- [中文更新日志](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CHANGELOG.zh-CN.md)

patch v1.2.0 新增 schema v3 last-known-good 激活、严格可选连接 JWT audit/enforce、
不可变 Relay snapshot、无副作用 simulation 与可选最小权限 Linux Control Agent。
schema v1/v2 继续有效，并保持连接认证关闭。

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

从 patch v1.1.0 升级 v1.2.0 时，第一次替换镜像/二进制仍使用原 schema v2（或 v1）。
另存一份 schema v3 候选：

```yaml
version: 3

# 原有 relay_servers、secure_tcp、mmdb 和 geo 部分保留在这里。

connection_auth:
  mode: off
  # 进入 audit 前再添加经过审核的 issuer/JWKS/introspection。
```

保持已有 WebSocket 设置不变。此时不要覆盖活动配置，也不要为了赶进度随意添加
authentication issuer。

## 3. 只拉取和检查，不启动

在 `.env` 设置新的不可变版本：

```dotenv
STARRY_VERSION=1.1.16-patch-v1.2.0
```

项目编排文件让 HBBS 与镜像内未经修改的 HBBR 使用同一个 Starry 版本，不再设置单独
更新的 HBBR 镜像版本。

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

继续使用旧 schema v2 或 v1，验证：

- 两个服务持续稳定并使用原密钥；
- 原生注册正常；
- 部署 API 时，登录后的原生 Secure TCP 正常；
- P2P 和原生 Relay 正常；
- 原生 Geo 与故障切换结果保持预期。

原生基线出现回归，应立即停止。

## 5. 先引入认证关闭的 schema v3

将候选文件安装为 `data/starry/config.yaml`，再调用已认证的 Control Agent
`POST /control/v1/runtime:reload` 操作并查看日志：

```sh
docker logs --tail 200 rustdesk-starry-hbbs
```

响应和日志必须报告新 generation、匹配的 source/effective digest 与成功 subsystem ack。
再次验证原生和所有此前启用的 WSS/mixed 路径。进程仍运行不等于验收；无效 candidate
会保留此前 last-known-good generation。

## 6. 分别接入 Agent 与连接认证

1. 以 `write_enabled: false` 和私有 listener 部署 Linux Control Agent；验证 mTLS
   CA/URI-SAN 与 service-JWT audience/azp/scope 拒绝。
2. 验证只读 status/Relay/config endpoint 与重复无副作用 allocation simulation；暂不开写。
3. 只在 staging 开写，演练 apply、rollback、HBBS outage、Agent restart、disk drift 与恢复
   阻断。
4. 部署兼容 client token issuer、公共 Ed25519 JWKS 与 mTLS introspection endpoint；HBBS
   保持 `connection_auth.mode: audit`。
5. audit 运行完整业务周期，并完成 native TCP、Secure TCP、WSS、直接 Relay、logout/
   revoke/disable/password-reset、key rotation 与依赖故障测试。
6. 单实例或小用户组 canary `enforce`；只有指标证据充分才扩大。UDP 发起保持 unsupported，
   且绝不能分配。

使用[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
中的完整清单。

## 功能回滚

如果只发生 WebSocket Signal 回归，原生行为仍正常：

1. 设置 `websocket_signal.enabled: false`；
2. 执行已认证的 `POST /control/v1/runtime:reload` 操作；
3. 确认管理响应和 HBBS drain 日志；
4. 关闭客户端 WebSocket 或撤销下发策略；
5. 重新验证原生注册、P2P、Secure TCP 和原生 Relay。

根据回滚策略关闭或停止使用公网 Nginx 路由。关闭功能后，已有 WSS session 会被 drain。

若连接认证回归，在本机受控把 `enforce` 改为 `audit` 并要求同步 reload ack；故意没有远程
一键认证 bypass。Agent 可独立停止或切回只读，HBBS/HBBR 继续使用最后活动配置。
operation 进入 `manual_intervention_required` 时，新写入必须保持阻断，直至核对 disk bytes
与 runtime digest。

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
启动 patch-v1.1.0 前，恢复文件必须是 schema v2 或 v1；patch-v1.1.0 不理解 schema v3。
v1.2 Agent audit/transaction state 应另行保留到 incident 关闭。

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
- 连接认证 request 绕过统一 gate，或预期合法 client 被意外拒绝；
- JWKS/introspection 故障疑似 fail open；
- Agent apply 没有匹配 runtime ack，或进入 `manual_intervention_required`；
- 两台客户端无法完成所需控制/数据测试。

发布检查和 CI 是有价值的 Release 证据，但无法替代在实际 DNS、证书、代理、网络和
客户端上的验收。
