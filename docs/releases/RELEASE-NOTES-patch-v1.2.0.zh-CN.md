# patch-v1.2.0 版本说明

[English](RELEASE-NOTES-patch-v1.2.0.md) | **简体中文**

patch-v1.2.0 在保留 Starry overlay 模式和官方 RustDesk wire protocol 的前提下，准备了
严格连接认证、可观测 Relay 分配与最小权限管理面。

本文是完整制品标签 `1.1.16-patch-v1.2.0` 的版本说明。

审核后的发布准备变更将 `RELEASE_STATUS` 设置为 `APPROVED`。只有精确提交的 source、
security、test、package、image 与 release-candidate jobs 全部成功，发布流程才会继续；
否则保持 fail closed。

## 组件与平台范围

- overlay 仍只修改 HBBS，HBBR 不做修改。
- 不包含账户、地址簿或设备 API。
- `starry-control-agent` 是独立可选管理二进制；远程 API 不代理 HBBS `21115`，也不
  提供 shell、任意文件、任意 URL、进程、Docker socket 或通用命令操作。
- v1.2.0 的可写 Control Agent 事务与所有正式承诺制品只支持 Linux amd64。发布范围包括
  `linux/amd64` 镜像、Linux x86_64 二进制/tar 和 amd64 DEB。ARM 仅尽力保持源码兼容，
  Windows 为非阻断实验构建；两者都不进入 v1.2.0 候选。

## 推荐 Docker 部署

继续推荐在 Linux amd64 主机上使用 Docker Compose。发布镜像包含 Starry HBBS、未经修改的
便捷 HBBR、`rustdesk-utils` 与可选 Control Agent；推荐的单机示例仍从匹配版本的官方
RustDesk Server 镜像运行 HBBR，使组件边界保持清晰。

- [GHCR 镜像页面](https://github.com/q1ngyang/rustdesk-server-starry/pkgs/container/rustdesk-server-starry)
- [容器镜像使用指南](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/CONTAINER.zh-CN.md)
- [推荐 Docker 部署指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)
- [单机 Compose 范例](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/examples/compose.yaml)
- [Control Agent sidecar 范例](https://github.com/q1ngyang/rustdesk-server-starry/blob/1.1.16-patch-v1.2.0/examples/control-agent/compose.yaml)
- [多节点部署指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Multi-Node-Deployment)

发布后请拉取不可变版本标签，不建议生产环境仅使用 `latest`：

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.0
```

## 配置生命周期

- schema v3 新增 `connection_auth`；schema v1/v2 继续有效并等价于认证关闭。
- runtime 状态现在记录 generation、精确 source digest、effective digest、激活时间与
  subsystem ack。
- 首次配置缺失、为空或无效时保持 upstream-compatible 行为；后续 reload 被拒绝时保留
  当前 last-known-good generation 并报告错误。
- candidate parse/validate 与 activation 分离；所有必需 subsystem 未确认同一 generation
  前，配置不会被报告为 active。

## 连接认证

- 统一 verifier 在有界 frame/protobuf 解析之后、target 查询、punch 记录、Relay 选择或
  消息投递之前执行。
- 覆盖原生 TCP、协商后的 Secure TCP 与 `/ws/id` WSS 上发起端
  `PunchHoleRequest` 和直接 `RequestRelay`。
- UDP 发起继续 unsupported，不进入任何分配路径。
- 官方 1.1.16 的原生被控端注册与心跳仍沿用上游 UDP 路径；这不承载上述发起请求，
  也不绕过发起端认证。需要禁用被控端 UDP 时，请使用 WSS 注册路径。
- 只接受 `alg=EdDSA`、`typ=at+jwt` 且带显式 `kid`、指向公共 Ed25519 JWK 的 access JWT。
  issuer、audience、token use、完整 scope、数值型 `user_id`、十进制 `sub` 绑定、正数
  `auth_version`、UUID `jti`、`iat`、`nbf`、`exp`、token 长度与签名均强制；该规则与
  Kessoku 实际签发的 wire 表示和仓库内字节 fixture 一致。
- JWKS rotation 原子且 last-known-good；远程 cache 的新鲜度写入与 JWKS digest 绑定的
  sidecar，restart/reload 不能重置 key age。metadata 缺失、digest 不匹配、keyset 超期、
  refresh 无效或已配置 introspection 故障均不得 fail open。
- 只要配置远程 JWKS 或 introspection endpoint，就必须同时使用 HTTPS、显式 CA、client
  cert、client key 与完全匹配的 DNS server name；这些内部 client 只允许 TLS 1.3，并禁用
  系统根证书和跳转。
  本地 JWT 失败绝不请求 introspection；introspection 使用 Kessoku 严格的仅 `token` DTO，
  返回 subject 还必须与本地已验证 JWT 一致。
- `audit` 只记录 would-allow/would-deny，不改变连接；`enforce` 使用稳定、与 target 是否
  存在无关的既有 protobuf 拒绝字段。
- `--must-login` 是部署层 enforce floor，配置 reload 和远程管理面都不能降低。

规范见[连接认证约定](../reference/auth/v1/profile.zh-CN.md)；字节级测试样本仍位于
[`contracts/auth/v1`](../../contracts/auth/v1)。

## Relay 可见性与模拟

- HBBS 向本地控制层发布不可变的 Relay/config/health snapshot。
- Relay 探测结果或就绪状态变化会推进 health snapshot identity，使 inventory、production
  分配与同一快照上的 simulation 可准确关联。
- 分配模拟与生产路径共享决策核心，但不会推进 rotation、改变 health/config、创建 Relay
  UUID、向 peer 投递消息或增加生产分配计数。
- 响应包含 generation/snapshot identity、规范化端点 facts、有序规则/Relay 决策、结果与
  warning。

## Control Agent v1

- 远程访问同时要求：client cert 链到配置 CA 且具有精确允许的 URI SAN；独立短期 EdDSA
  service JWT 绑定 Agent instance 与请求 scope。
- 固定 API 提供 capabilities、status、Relay inventory、分配模拟、配置 schema/state/
  validate、配置事务、operation 查询、history、rollback 与带审计 runtime reload。
- 每个已知 HTTP action 都固定映射到精确 scope，并在读取/分配 body 前完成 mTLS/JWT
  principal 验证；未知 action 保持 404。
- HBBS loopback bridge 只接受有界 `STARRYCTL/1` frame，其中的 secret 从绝对路径、普通文件、
  mode 0600 的 token file 读取并做 constant-time 比较；legacy 文本命令 dispatcher 在 runtime
  不可达。
- `config.write_enabled` 默认 `false`；该 profile 不公布写 capability，并在认证后对
  plan/apply/rollback/reload 返回 404。
- apply plan 绑定 instance、caller、精确字节 ETag、runtime generation、candidate digest
  与过期时间；mutation 强制 `If-Match` 与 idempotency key。
- resident plan 同时限制数量和总字节；idempotency replay 绑定 Agent instance 与已认证 caller。
- intent、operation、idempotency 结果、revision、recovery material 与脱敏 audit 均持久化；
  raw JWT 与 raw idempotency key 不落盘。
- candidate 发布依次执行临时文件 create/write/fsync、原子 rename、目录 fsync 与同步 HBBS
  activation ack。失败时恢复原 bytes 与 runtime；恢复结果不确定则进入
  `manual_intervention_required` 并阻断后续写入。
- 启用写入的 Agent 会在启动时确认现有 managed config 是单 hard-link 普通文件，且由 Agent
  的 effective UID 与 primary GID 所有，避免保留 owner 的原子替换到 apply 后期才失败。

详见 [`contracts/control/v1/openapi.yaml`](../../contracts/control/v1/openapi.yaml)及
[Control Agent 指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)。

## 升级与灰度

1. 备份 HBBS 数据、配置精确 bytes、identity key 和旧镜像/包；暂不启动 Agent。
2. 保留现有 schema v1/v2 配置升级 HBBS，并验证当前已使用的 native、Secure TCP、WSS 与
   mixed 路径。
3. 改用 schema v3 且保持 `connection_auth.mode: off`；reload 并确认 activation ack 与
   generation。
4. 在私有管理路径以只读模式部署 Agent，验证 mTLS、service-JWT scope、status、Relay
   inventory 与 simulation。
5. 先在 staging 演练写事务和 rollback，再在其他环境设置 `write_enabled: true`。
6. 部署兼容的 client JWT issuance、JWKS 与 mTLS introspection；保持 `audit` 至少一个完整
   业务周期，并解释所有 would-deny。
7. 对单实例或小用户组 canary `enforce`；真实 client P2P/Relay/WSS/mixed 与依赖故障测试
   通过后才扩大。

## 回滚

- 认证正常回滚是在本机受控修改 schema v3，把 `enforce` 改为 `audit` 并获得 reload ack；
  Agent 故意没有特殊“一键关闭认证”接口。
- 可停止 Agent 或切回只读而不停止 HBBS/HBBR；数据面继续使用最后活动 HBBS 配置。
- HBBS 回滚 patch-v1.1.0 前必须恢复升级前 schema v2 bytes；v1.1.0 不理解 schema v3。
- operation 若报告 `manual_intervention_required`，不得重试写入；先根据持久 recovery/audit
  记录核对磁盘 bytes 与 HBBS runtime digest。

## 发布前验证状态

截至 2026-08-20，隔离的本地发布候选已完成：

- Rust `1.97.1` 下 76 个库测试、全部 binary check 与协议/集成 target 通过，包括确定性
  token/protobuf mutation corpus、native/Secure TCP/WSS 的两类认证请求、local control、
  Control Agent 原子事务、mixed HBBR，以及 1,000 个已注册 idle WSS session；
- 官方 RustDesk 1.4.9 Linux 客户端原始二进制
  `sha256:7244ba47d14225a7aa1ae2d6802925b7680c3f2dd16e79c28a2f6dd4066e3687`
  完成 `audit→enforce`：audit 中无效 token 仍建立桌面且增加 would-deny，enforce 中合法
  native TCP 与 WSS 会话通过；missing/expired token、注销、禁用、密码重置前 token 与退休
  key 均拒绝，密码重置后 token 通过；
- 同一官方客户端在 enforce 下完成 WSS 发起端→原生被控端与原生发起端→WSS 被控端两个
  mixed Relay 方向，HBBR UUID 双端配对并显示远端桌面。官方 1.1.16 的原生被控端注册/心跳
  仍走 UDP；控制端连接发起走 TCP/Secure TCP 并已认证，UDP 发起消息无响应、无分配；
- Kessoku mTLS JWKS/introspection E2E 覆盖 current/previous key overlap、新 key、旧 key
  退休、logout/disable/password-reset 与 fail-closed。修复 HTTP idle pool 后连续 12 次、6 分钟
  的 30 秒 JWKS refresh 均成功，key age 持续更新；
- 一个 HBBS 加六个独立 HBBR 的本地七容器拓扑完成第一 Relay 故障与恢复：基线、故障、恢复
  的快照 ID 分别为 `health-14`、`health-50`、`health-80`，simulation 依次选择 relay1、
  relay2、relay1，且不会改变生产 rotation/state；
- SQLite online backup、Starry identity/config/token、Kessoku config/key 的 checksum 备份在
  全新网络恢复；数据库 integrity、用户/auth version、token introspection、HBBS identity、
  verifier readiness 与 activation digest 均一致；
- 使用同一数据目录完成已发布 patch-v1.1.0 → 候选 patch-v1.2.0 → patch-v1.1.0：候选读取
  schema v3 并返回 activation ack，恢复 schema v1 配置后旧版重新启动，三阶段服务器身份
  hash 完全相同；因此旧镜像回滚必须同时恢复旧版可读的配置快照；
- 本地最终 amd64 静态四二进制、四个 DEB、固定 Debian 安装/runtime 和发布 Dockerfile
  四命令/config-generation smoke 通过。最新本地预发布镜像 ID 为
  `sha256:0995b73a19a64fbdb6204082b78907f50e1210d4318b664e3080dc31eab0c155`；它不是将来
  GHCR 发布 digest，最终 digest 由干净发布 workflow 产生；
- 文档/contract/Compose/workflow/format、overlay 双次幂等与 lockfile/metadata 检查通过。
  actionlint 1.7.12、Gitleaks 8.25.1 与 Syft 1.50.0 均由固定 checksum 安装；历史与当前
  候选 secret scan 为 0 finding，源码 SPDX SBOM 已生成；
- 固定 `cargo-audit 0.22.2` 与固定 RustSec 数据库提交审计 401 个依赖：0 vulnerability、
  0 unsound；仅披露 upstream core `sodiumoxide 0.2.7` unmaintained warning。此前密封的
  Codex Security 静态审计无 high/critical，主要 Starry-owned finding 已修复并具回归测试。

## 发布流程与发布后门禁

- 用户已于 2026-08-20 确认本次中英文文档与新特性说明；本次审核后的发布准备提交将
  `RELEASE_STATUS` 设置为 `APPROVED`。
- 精确提交必须在干净 GitHub Actions 中通过 source/security/test/linux/DEB/image/package/
  release-candidate jobs；随后 publish job 才会创建 Release/GHCR 镜像，并签署最终
  linux/amd64 SBOM、provenance、Sigstore bundles 与 `SHA256SUMS`。
- 发布不可变 Starry tag 后，Kessoku 必须把本地候选 contract 标记替换为该 tag/digest
  （`status: PINNED`）并在其发布前重跑干净的跨项目测试。

本地七容器故障域、轮换和恢复演练不等于用户目标生产网络、存储、备份介质、调度器与证书
系统的验收。目标环境仍应在正式流量扩大前重复同一 runbook；这是部署上线门禁，不冒充为
本 patch 的本地候选证据。ARM 与 Windows 兼容性明确不阻断 patch-v1.2.0。
