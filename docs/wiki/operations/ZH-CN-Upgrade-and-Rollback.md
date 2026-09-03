# 版本升级与回滚

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback) | **简体中文**

升级同时涉及两个版本：官方 RustDesk Server 基线和 Starry patch。例如
`1.1.16-patch-v1.3.1` 表示官方服务端 `1.1.16` 加 Starry patch `1.3.1`。生产环境
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

- [patch-v1.3.1 发布候选说明](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/RELEASE-NOTES-patch-v1.3.1.zh-CN.md)
- [中文更新日志](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/releases/CHANGELOG.zh-CN.md)

patch v1.3.1 在冻结 v1.3.0 的候选 Relay 探测、双端 RTT/jitter/loss 评分、可信 HBBR
负载遥测、签名 FastCompat 授权和匹配 ACK Profile Activation Lease 之上，增加角色授权
FastMedia Relay UDP、telemetry schema 2、schema v5 和 SP1 pairing。
官方客户端与 schema v1-v4 保持原注册和可靠 Relay；FastMedia/pairing 独立且默认关闭。
真实回退/重入门禁通过前候选继续 BLOCKED。

## patch-v1.3.1 升级与降级

更换容器前，把完整 Control 状态绑定到同一绝对 `STARRY_PERSIST_ROOT`，每个 Relay 绑定到
自己的绝对 `RELAY_DATA_DIR`。把 `hbbs`、`config`、`control/state`、
`control/identity`、`control/generated`、`control/shared`、`relay-secrets` 及每个 Relay
`starry/enrollment` 作为一套保留权限备份，并在隔离主机验证。原 Relay 与身份副本绝不能
同时上线。只有显式挂载相同目录时 `pull`、`force-recreate`、`down`/`up` 才保持身份；
`down -v`、相对路径改变、overlay/tmpfs 状态或 host identity 不匹配都必须预检失败。

升级顺序：

1. FastMedia 策略保持关闭，先滚动 HBBR v1.3.1，保持可靠 TCP/WS 路径不变。验证认证
   telemetry schema 2、UDP listener 健康、普通 Native/WSS/mixed Relay 和官方客户端。
2. 使用原 schema v4 或两个 Fast 开关均 false 的 schema v5 滚动 HBBS；验证精确 Relay
   Quality v1 digest 和普通 GEO/failover。
3. 滚动 Control Agent，验证 schema/OpenAPI fixtures、SP1 capability 和只读 inventory。
   既有身份只能通过显式审核的 pair/adopt 操作接入，`pair` 不得覆盖。证书通过 DNS 使用时，
   用 `--tls-server-name` 传入 Kessoku allowlist 中完全相同的名字；rotate 必须保留既有
   Agent listen、local-control address、大小上限与写策略。
4. 先 canary FastCompat，再在有界 Relay/Akari 群组 canary FastMedia；bind 失败、UDP
   block、listener restart 或限流都必须保留可靠桌面会话。

schema v5 只新增：

```yaml
version: 5
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
    relay_max_datagram: 1200
```

每个 eligible Relay 还要在认证 `/ws/telemetry` endpoint 旁声明
`fast_media_udp_port`。新鲜 telemetry 未报告同端口 protocol 1 且 healthy 前不得启用。

v1.3.1→v1.3.0 时，先设置 `fast_media_v1_enabled: false` 并取得同步 activation ACK，再
预览/导出：

```console
starry-control-agent config downgrade --to-schema 4 --preview
starry-control-agent config downgrade --to-schema 4 \
  --output /safe/config-v4.yaml
```

存在 FastMedia authorization、allocation、active stream、尚未过期的最后 grant，或任一
Agent/Relay 证书剩余不足九十天时，命令拒绝。进入回退窗口前先轮换证书。导出 no-clobber，
只删除 v5 字段。审核 digest 并激活 schema v4 后，依次回滚 HBBS、使用非 secret
`relay-compat.env` 的 HBBR、Control Agent。patch-v1.3.0 继续读取 Agent v1 YAML/PEM/JWKS
和普通 telemetry secret-file，只忽略、不能删除 pairing/enrollment 状态。重新升级 v1.3.1
会复用同一身份，并在恢复 FastMedia 前重新检查 UDP 健康。
兼容解析器会保留公开 `KEY` 末尾的 Base64 padding。

Kessoku 降级必须作为协调兼容变更执行。Kessoku v3.0.7 冻结 config schema ≤4 和
telemetry schema 1，不能直接读取 schema-v5/telemetry-v2 Starry v1.3.1 inventory。
启动 Kessoku v3.0.7 前，先激活导出的 schema-v4 配置并把 Starry 回滚到 v1.3.0；升级时
反向执行，恢复 Kessoku v3.0.8 与 Starry v1.3.1、验证新鲜 telemetry-v2 inventory 后，
才重新启用 FastMedia。

以下编号步骤是原 v1.3.0 commissioning 流程，仍适用于 Relay Quality/Profile activation。

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

从 patch v1.2.0 升级 v1.3.0 时，第一次替换镜像/二进制仍使用原 schema v3（或更早）。
另存一份 schema v4 候选：

```yaml
version: 4

# 原有 relay_servers、secure_tcp、mmdb 和 geo 部分保留在这里。

# 原样保留已有 connection_auth 与 websocket_signal 设置。

relay_quality:
  enabled: false
  # 原有与官方客户端基线通过后再启用。

fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
  # 质量、鉴权和安全信令灰度均通过后才启用。
```

此时不要覆盖活动配置，也不要为了赶进度弱化已有 authentication 模式。

## 3. 只拉取和检查，不启动

在 `.env` 设置新的不可变版本：

```dotenv
STARRY_VERSION=1.1.16-patch-v1.3.0
```

项目编排文件让 HBBS 与 HBBR 使用同一个 Starry 版本。HBBR 保留上游字节转发路径，
仅增加有界公开探测与认证遥测通道；不再设置单独更新的 HBBR 镜像版本。

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

继续使用旧 schema v3（或更早），验证：

- 两个服务持续稳定并使用原密钥；
- 原生注册正常；
- 部署 API 时，登录后的原生 Secure TCP 正常；
- P2P 和原生 Relay 正常；
- 原生 Geo 与故障切换结果保持预期。

原生基线出现回归，应立即停止。

## 5. 先引入 Relay 质量关闭的 schema v4

将候选文件安装为 `data/starry/config.yaml`，再调用已认证的 Control Agent
`POST /control/v1/runtime:reload` 操作并查看日志：

```sh
docker logs --tail 200 rustdesk-starry-hbbs
```

响应和日志必须报告新 generation、匹配的 source/effective digest 与成功 subsystem ack。
再次验证原生和所有此前启用的 WSS/mixed 路径。进程仍运行不等于验收；无效 candidate
会保留此前 last-known-good generation。

## 6. 单独灰度 Relay 质量

先升级 HBBR，再在 HBBS 激活质量功能。inventory 必须显示
`relay_probe_protocol: 1`、`relay_load_protocol: 1`、新鲜 telemetry，并且每个质量 Relay
都有唯一 health endpoint；legacy HBBR 必须列入 `legacy_fallback_relays`，版本字符串不能
作为能力证据。至少保持两个兼容候选 Relay 在线，再只在一个中心或客户端群组启用
`relay_quality`，验证：

1. 官方客户端仍只接收并使用原有 `relay_server`；
2. 兼容 Akari 的 stage 1 只收到 GEO primary；primary 良好时不探测其他 HBBR，任一端
   报告质量不佳时双方同时收到其余候选；
3. HBBS 在原字段与可选 decision 扩展中返回同一个最终 Relay；
4. Kessoku/Control Relay 状态提供能力版本、telemetry 观测年龄/stale 状态，以及不含客户
   标识的 accepted/late/invalid/binding-mismatch 计数；
5. P2P 成功会取消 allocation，force-auto/WSS/对称 NAT 在最终 decision 前不连接 HBBR，
   官方对端会得到明确的单端 partial 结果；
6. 丢包、过载、健康过期、阶段乱序、重复/迟到 report 与限流下的选择或 fallback 均确定；
7. 同一网络前缀对在 hysteresis 阈值内重连时不会抖动切换。

客户端绝不能访问 Control Agent。若部署的 Akari 协议版本与发布契约不一致，保持
`relay_quality.enabled: false`；canonical protobuf SHA-256 不匹配 FROZEN 合约时也必须关闭。
不得依据尚未冻结的分支开始部署 Akari wire 实现。

## 7. 分别接入 Agent 与连接认证

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

## 8. 在 FastCompat 前灰度 Profile activation

先升级所有目标 HBBS 及其 Control Agent，确认 `profile_activation_lease: 1`；再升级
Kessoku，使其按 HBBS `instance.id` 保存 route lease、在签发实例调用 `/peers:verify`，并
脱敏 activation ID/public key/lease。此时仍不启用 Akari 切换。

只有 Akari 客户端 state machine 能在成功 Ready ACK 精确匹配 activation ID/epoch 且包含
32 字节 lease 与非零 generation 前保持旧 Profile 已提交，才开始灰度。覆盖 Native
UDP/TCP、WSS 旧 reader 退出、乱序消息、A→B→A、官方旧客户端和至少两个 HBBS 节点。
只有 stale/rate/capacity/TTL 计数和 HBBR session/disconnect 压力保持基线内才扩大。

完整协议与发布约定见
[Profile Activation Lease v1](../../reference/PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)。

## 9. 最后灰度 FastCompat 授权

配置中不提供 `allow_fast_media_v1`；patch-v1.3.0 将其固定为 false，也不发布 FastMedia
Relay 传输。在 Relay 质量、鉴权严格 allow 和 Secure TCP/WSS 灰度都通过后，才通过一项
独立计划变更设置 `fast_mode.relay.fast_compat_enabled: true`。

验证：

1. `GET /control/v1/capabilities` 返回 `fast_relay_authorization: 1`，且
   `/relays.fast_relay` 计数存在；
2. 官方客户端及未取得质量 offer 的 Akari 不收到授权，标准 Relay 流程仍可完成；
3. Akari 使用现有 HBBS 公钥验签，核对会话 UUID、有效期、码率，并确认
   `allow_fast_media_v1: false`；
4. 同一最终 Relay 质量决定之后，目标端 `RequestRelay` 与控制端 `RelayResponse` 包含
   完全相同的签名字节；
5. 同会话重试复用完全相同的字节且不延长有效期；来源/目标绑定改变时不签发；
6. Kessoku audit 与服务日志只包含策略/计数，不包含签名密钥、连接令牌、会话 UUID 或
   签名授权。

使用[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)
中的完整清单。

## 功能回滚

若 Profile activation 回归，先停止新的 Akari 切换并保留各客户端最后已提交 Profile；
使用各签发 HBBS 自己的 lease/generation best-effort 发送 `DeactivatePeer`，再等待至少
45 秒并加上 WSS idle/drain。服务端状态仅在内存中；不得删除 peer record 或轮换 public
key。旧服务端不返回匹配增强 ACK，因此合规客户端会 fail closed 到原 Profile。

若 FastCompat 回归，先设置 `fast_mode.relay.fast_compat_enabled: false` 并取得同步 reload
ack。新会话会立即停止获得授权并继续标准可靠 Relay 路径。等待超过配置的授权 TTL 后，
才可视为此前签发授权全部过期；已有 HBBR 数据流不会被强制迁移。

若只有分阶段协调回归，先设置 `relay_quality.strategy: eager` 并取得同步 reload ack；这会
保留冻结 v1 wire，同时让新 allocation 恢复 eager-all-candidates。若 Relay 质量整体回归，
设置 `relay_quality.enabled: false`，执行同步 runtime reload，
确认新分配只使用原有 Geo/failover Relay 选择。已有 Relay 字节流不会迁移，应按事故
策略自然 drain 或重连。回滚 HBBR 镜像前至少等待一个 `report_timeout_ms`。若只回滚一台
HBBR，先将其移入 `legacy_fallback_relays` 并 reload，确认它不再是质量候选，等待 in-flight
offer 结束后再替换；HBBR 撤销能力后才回滚 HBBS。

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

回滚后重复原生及适用的 API/Relay 验收。启动 patch-v1.2.0 前必须恢复不含
`fast_mode` 和 `relay_quality` 的 schema v3（或更早）；patch-v1.2.0 会拒绝 schema v4。
启动 patch-v1.1.0 前恢复 schema v2 或 v1。Agent audit/transaction state 应另行保留到
incident 关闭。

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

HBBR 包从固定官方 revision 构建，保留上游字节转发路径，并增加有界公开探测与认证
遥测通道。回滚需要保留旧包或仓库快照，不能假定包缓存仍然保存它们。

独立二进制应使用带版本的文件名，以原子方式更新服务软链接或路径。记录摘要并备份前，
绝不能覆盖唯一一份已知可用二进制。

## 出现以下情况应停止并回滚

- Starry 配置被拒绝或意外恢复上游行为；
- 现有密钥改变或持久化文件消失；
- 原生注册或原生 Relay 回归；
- 原先可用的已认证客户端 Secure TCP 失败；
- 所需传输没有符合条件的 Relay；
- 质量 offer/decision 与原有 Relay 字段不一致；
- 超过文档阈值后 WSS 健康仍未 Ready；
- 连接认证 request 绕过统一 gate，或预期合法 client 被意外拒绝；
- JWKS/introspection 故障疑似 fail open；
- Agent apply 没有匹配 runtime ack，或进入 `manual_intervention_required`；
- 两台客户端无法完成所需控制/数据测试。

发布检查和 CI 是有价值的 Release 证据，但无法替代在实际 DNS、证书、代理、网络和
客户端上的验收。
