# patch-v1.3.1 发布候选说明

[English](RELEASE-NOTES-patch-v1.3.1.md) | **简体中文**

patch-v1.3.1 基于已冻结 patch-v1.3.0 commit
`abf1dbcfdf7c4384f8c7ac34724089932a1bc58c` 开发。它实现 Akari
FastCompat/FastMediaV1 Relay 的 Starry 侧和 Starry Pairing v1，并保持官方客户端、旧
Akari、手工部署及冻结 Relay Quality v1 契约兼容。

**发布状态：BLOCKED。** 在真实 Akari↔HBBS↔HBBR 集成证明双角色授权/绑定、加密转发、
可靠回退与自动重入之前，不得创建 tag 或宣称冻结。只有该门禁和完整 CI 对一个精确
候选 commit 全部通过后，发布工作流才会写入最终 commit 与契约 digest。

## Fast Relay 与 FastMedia

- Relay Quality 关闭、兼容候选不足两个、超时或 legacy fallback 时，HBBS 现在也能为
  自己的最终 Relay 签名。若 Relay Quality v1 已形成 decision，它仍是权威；普通
  `relay_server` 与签名 `relay_server` 必须完全一致。
- `FastRelayAuthorization.version = 1` 保留字段 1–6，追加 tag 7–12：UDP protocol、
  最终 Relay、UDP 端口、新鲜 16 字节 allocation ID、datagram 上限和端点角色。
  controller 固定为 1，target 固定为 2。
- FastMedia 会话得到两份不同 Ed25519 combined grant；旧六字段 FastCompat 继续兼容。
  两个开关均默认关闭，任一失败都保留可靠 HBBR 数据流。
- HBBR 实现 32 字节 AKR1 封装和 Hello/Cookie/Bind/Bound/Media 状态机：来源绑定无状态
  cookie、角色授权验签、allocation/session/Relay/source tuple 固定、双角色就绪门禁、
  AKF1 明文前导校验、剥离 AKR1 后原样转发加密 AKF1。
- 同角色迁移需要新 cookie，并原子撤销旧 tuple。授权/datagram 大小、过期、半绑定/
  idle/绝对生命周期、allocation 数、清理工作、replay window、rebind 及每角色/每 IP/
  全局流量均有硬上限。
- 签名码率是编码源上限；HBBR wire 上限为 `ceil(source × 1.45)` Kbit/s，每角色 burst
  不超过 `max(256 KiB, 50 ms wire allowance)`。

## Relay Quality v1 修复

冻结 protobuf、tag、评分、分阶段策略、遥测解释、迟滞、reason code、隐私与 digest
均不变。只修复 native 初始响应 route：当原 controller route、target IP 与 ID、
allocation、stage/token、target role、候选集和 generation 全匹配时，
`PunchHoleSent`/`LocalAddr` 可以使用新的 target 来源端口。顶层 report 和 controller
route 仍精确绑定；相同重复幂等，冲突重复拒绝。

## schema v5、遥测与 Control API

schema v5 新增互相独立且默认关闭的 `fast_compat_enabled`、
`fast_media_v1_enabled`、`relay_max_datagram` 和每 Relay `fast_media_udp_port`。
FastMedia 授权要求新鲜认证 telemetry schema 2、明确
`fast_media_relay_udp = 1` 和健康 UDP listener；公开客户端 probe 不暴露这些负载/运行态。

认证 HBBR telemetry 增加 capability、UDP 健康、active allocation/stream，以及有界
cookie/bind/grant/role/session/allocation/rebind/forward/drop/rate/replay/expiry/listener
计数。Control API 只暴露 typed Relay 状态和有界聚合，绝不返回完整客户端地址、UUID、
allocation ID、nonce、stage token、grant、secret 或媒体。

## Starry Pairing v1

- `starry-control-agent pair`、`adopt`、`rotate` 只从 stdin 或 mode-0600 文件读取短期
  单次 `SP1` 代码。Agent 本地生成私钥/CSR，校验 Broker origin/SPKI pin、响应绑定、
  key/certificate 匹配、CA 签名与证书有效期，并且不覆盖不相关既有身份。
  显式 `--tls-server-name` 会作为经过校验的 DNS SAN 写入 CSR；首次配对中断后复用
  durable pending instance UUID，rotate 保留已经校验的 Agent-v1 运行设置。
- Relay enrollment 由现有 mTLS/JWT Control Agent API 授权。Kessoku 可代理领取，但不能
  选择 Relay endpoint、池、secret 或配置。prepare/complete/health-activate/revoke/list/get
  有界且幂等；health activation 还会把成功 config operation ACK 绑定到精确 generation，
  并重新读取 HBBS inventory 后才把 enrollment 标为 active。撤销只清理精确当前凭据；
  相同 node 的后续 enrollment 只能清理匹配且已 revoked/expired 的前任。
- `starry-relayctl enroll` 在本机生成 Relay node key，不改变上游 `hbbr` CLI。每 Relay
  获得独立 telemetry secret、证书、批准运行配置和只指向 secret 文件的非 secret
  `relay-compat.env`。
- 手工 Agent YAML/mTLS/JWT 及手工 HBBR 公钥/secret-file 保持支持；已有数据面不依赖
  Broker 在线。

## 持久部署边界

所有容器 Control identity/state/shared token/generated config、每 Relay 材料和兼容快照
都位于 `STARRY_PERSIST_ROOT`；Relay enrollment 位于 `RELAY_DATA_DIR`。配对/启动会拒绝
overlay/tmpfs 容器状态、不安全路径类型或权限、缺失显式 mount 和 Relay host identity
不匹配；镜像不再声明匿名 `/root` volume。

继续挂载同一宿主根时，`pull`、`force-recreate`、`down`/`up` 保持身份；`down -v`、
挂载另一个相对目录和并发身份克隆属于 fail-closed 操作错误。原生/DEB 使用
`/etc/rustdesk-server-starry` 与 `/var/lib/rustdesk-server-starry`；普通 package
upgrade/downgrade 不覆盖身份。
Relay-only Compose 默认把 enrollment 强制开关设为 `1`；保留的手工公钥模式必须显式
设为 `0`，而且不得用于在状态缺失时重启曾经 enrollment 的 Relay。
兼容文件解析器只在每个 allowlist assignment 的首个分隔符处分割，因此不会丢失公开
RustDesk `KEY` 的标准 Base64 padding。

## 升级与回滚

依次升级 HBBR、HBBS、Control Agent，最后升级兼容 Akari。认证 schema-2 telemetry 和
可靠会话回归通过前，两个 Fast 开关保持关闭；先 canary FastCompat，再启用 FastMedia。

v1.3.1→v1.3.0 前先关闭 FastMedia，运行
`starry-control-agent config downgrade --to-schema 4 --preview`。命令通过本地 Agent 查询
active allocation、authorization、stream 和最新 grant expiry；`--runtime-state` 是显式
审计的离线 override。未完全 drain 或任一 Agent/Relay 证书剩余不足九十天时拒绝导出。
输出只删除 v5 字段，且不覆盖已有目标。

patch-v1.3.0 可以读取配对生成的 Agent v1 YAML/PEM/JWKS。已 enrollment HBBR 使用
`relay-compat.env` 提供公开 `KEY` 与现有 telemetry secret-file 路径，保留普通
Native/WSS Relay 和 telemetry。旧版只忽略、不删除 enrollment/FastMedia 状态；升级回
v1.3.1 后复用身份，并重新验证 UDP 健康。

## 兼容矩阵

| 客户端/HBBR 组合 | 结果 |
| --- | --- |
| 官方客户端 + 官方/legacy HBBR | 普通 P2P/LAN/Relay 不变；无 Fast grant 或 UDP candidate。 |
| 旧 Akari + patch-v1.3.1 HBBR | 六字段 FastCompat 可用，未知字段忽略，可靠 Relay 保留。 |
| 新 Akari + legacy HBBR | HBBS 可为最终 Relay 签 FastCompat，但绝不签 FastMedia；UDP capability 为 null/fail-closed。 |
| 新 Akari + 健康 patch-v1.3.1 HBBR | 策略/鉴权通过时可用角色授权与 AKR1；可靠 Relay 始终连接。 |
| 官方/新 Akari 混合 | 官方端忽略 tag 64；普通 HBBS 最终 Relay 仍可互通。 |
| v1.3.1 enrollment + v1.3.0 runtime | Agent v1 凭据和普通 Relay/telemetry 保留；pairing 自动化与 FastMedia 被忽略。 |

## 冻结契约候选

Relay Quality v1 仍是既有冻结依赖。唯一的 patch-v1.3.1
`CONTRACT-RELEASE-SUMMARY.json` 以逐文件 SHA-256 冻结 Control OpenAPI、config schema
v5/UI schema、`capabilities`/`relays`/`status`、Relay enrollment、SP1、十二字段 Fast Relay
授权、AKR1、telemetry v2 schema/fixture 和 downgrade drain-state schema。source binding
是包含该摘要的 Git commit。schema 支持只用 `capabilities.config_schema = 5` 表达，并附带
supported/active version 与 schema digest。Kessoku v3.0.8 可以 pin 推送后的精确契约候选
commit，绝不能 pin dirty worktree；`RELEASE_STATUS` 仍为 `BLOCKED` 时也不能把它当作运行时
发布批准。

## 验证状态

仓库已加入 schema 默认/门禁、服务端 fallback grant、角色交换/篡改/过期、cookie、双端
bind、AKF1 转发/replay、来源端口 rebind、速率/生命周期、认证 telemetry/Control JSON、
SP1 过期/重放/用途/key/digest/pin、中断安装恢复、enrollment 幂等、持久 mount 和无副作用
downgrade 测试。真实 HBBR 子进程测试覆盖双角色，并证明 UDP 工作不破坏可靠 TCP。

本地候选还通过了 140 个非 SQLite lib 测试、单独串行 SQLite 慢测、11 个 HBBR binary
测试、24 个定向集成测试（另有一个显式 ignored 的负载门禁）以及单独运行的 1000 WSS
发布门禁；CI 所属文件 rustfmt、`cargo check --all-targets`、`cargo clippy
--all-targets`、native/amd64-musl release build、本地镜像命令与持久化 smoke、四包 DEB
可复现构建、固定 Debian 镜像安装及 HBBS v1.3.1→v1.3.0→v1.3.1 身份往返均通过。
干净官方 commit 连续应用 overlay 两次的整树文件 digest 均为
`a83db4b81c4dc2867785a36d869bba09afc2b677e085d47a9cb686537f48a1e5`。

后续隔离、非发布 staging 使用同一 `KESSOKU_DATA_DIR` 对接 Kessoku v3.0.8，通过 SP1
Control 证书轮换、重启后 mTLS/JWT inventory、健康门控 Relay 重新 enrollment、修正后的
容器重建，以及 relative content/metadata manifest 完全相同的停机备份恢复。
协调执行 v3.0.8/v1.3.1 → v3.0.7/v1.3.0 schema-v4 → v3.0.8/v1.3.1 后，registry、
HBBS identity、生成的 Agent-v1 identity、普通 Native/WSS Relay 与新鲜 telemetry 均保留。
Kessoku v3.0.7 直接读取 Starry v1.3.1 inventory 会因前者冻结 config schema ≤4 和
telemetry schema 1 而被正确拒绝。该 staging image 来自 pre-review worktree，只是诊断
证据；所有适用操作仍须对最终精确干净 release commit 重跑。

以下仍是对同一精确候选的发布阻断项：

- Native、WSS、mixed 上真实 Akari↔HBBS↔HBBR 自动回退与重入；
- 真实 UDP block、HBBR restart、300–1200 ms 迁移、整形丢包/超限及设备长时 soak；
- 生产 PKI 证书轮换、九十日回滚窗口前 enrolled Relay 轮换，以及多主机迁移、身份克隆
  和 down-volume 演练；
- 修复并复验 Kessoku v3.0.8 Relay adapter 当前遗漏契约字段
  `websocket.process_instance_id` 的问题；
- 精确干净 commit 上的托管 CI、RustSec/历史 secret scan、SBOM/attestation、交叉构建工件
  可复现性及完整 Docker/DEB/native 配对与 Relay 交叉升级矩阵。
