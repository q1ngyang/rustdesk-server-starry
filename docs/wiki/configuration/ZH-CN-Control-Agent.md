# Control Agent

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Control-Agent) | **简体中文**

`starry-control-agent` 是面向单个本机 Starry HBBS 的可选 Linux 管理组件。Kessoku 或
其他 controller 通过强制 mTLS 和细粒度 service JWT 的 HTTPS 访问 Agent；只有 Agent
使用 `127.0.0.1:21115` 上有界的 loopback `STARRYCTL/1` 协议访问 HBBS。

Agent 不是账户 API，也不是 HBBS 数据面的必需组件。停止 Agent 只会移除管理访问，不会
禁用 HBBS 最后活动配置。

## 安全边界

每个远程请求同时要求：

1. client cert 链到 `tls.ca_file`，且 URI SAN 精确命中一个
   `allowed_client_uri_sans`；
2. 来自独立 `service_jwt.jwks_file` 的 EdDSA service JWT，lifetime 最多五分钟，并具有
   预期 issuer、`azp`、请求 scope 与 audience
   `urn:starry-control:<instance_id>`。

连接 JWT key 与 service JWT key 必须分离。API surface 由
[`contracts/control/v1/openapi.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/contracts/control/v1/openapi.yaml)固定；不提供
任意命令、任意路径、Docker/systemd 控制、URL fetch、shell 或裸 `21115` proxy。

同机 controller 应让 Agent 监听 host loopback；远程 controller 应使用 firewall 限制的
私有管理地址。绝不能通过公网 RustDesk 或 reverse-proxy listener 发布 `21120`。

## Linux 安装

Linux archive/container 和 `rustdesk-server-starry-control-agent` DEB 包含 Agent。DEB 安装
后不会自动 enable systemd service。v1.3.0 不发布 Windows Agent，因为原子配置事务只在
Unix filesystem 上属于发布支持范围。

从 [`config/control-agent.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/control-agent.example.yaml)开始：

```yaml
version: 1
instance_id_file: /var/lib/rustdesk-server-starry/control-agent-instance-id
listen: 127.0.0.1:21120
local_control:
  address: 127.0.0.1:21115
  token_file: /etc/rustdesk-server-starry/local-control.token
config:
  write_enabled: false
  path: /etc/rustdesk-server-starry/managed/config.yaml
  backup_dir: /var/lib/rustdesk-server-starry/config-history
  max_bytes: 1048576
```

把 server certificate/key、client CA 与 service public JWKS 安装到配置路径。Agent service
user 只需读取这些文件，只对 managed config/state directory 具有读写权，且不应访问 Docker
socket 或 host service-control interface。

HBBS 与 Agent 必须共享一份独立的本地控制 token。DEB 会以 mode `0600` 创建
`/etc/rustdesk-server-starry/local-control.token`，并通过
`STARRY_LOCAL_CONTROL_TOKEN_FILE` 配置 HBBS。容器部署时，创建仅含 32–256 个
base64url 字符的 `secrets/local-control.token`，只允许 Agent numeric UID 读取，并把同一
只读文件挂载给两个服务。文件缺失、权限过宽、格式错误或值不一致都会 fail closed；
远程 Control API 不接受该 token。

启用写事务时，现有 managed config 必须由 Agent service UID 与 primary GID 所有。仅把
root-owned 文件设为 group-writable 并不够：原子 rename 会创建新 inode，而最小权限 Agent
若不能保留精确 owner，会在启用写入时拒绝启动。DEB 将受管文件设为
`rustdesk-starry:rustdesk-starry`、mode `0640`；container bind mount 必须使用 Compose 环境中
的 numeric UID/GID。配置文件不得允许 group/other 写入，所有 parent component 都必须是
真实、受约束的目录；每次事务会把替换操作绑定到读取源 bytes 时看到的 parent device/inode。
state root 及其生成的子目录必须由 Agent 所有且 mode `0700`，持久 JSON/YAML 必须是 mode
`0600` 的单链接普通文件。TLS key、Agent YAML 与 service JWKS 仍由 root 所有，Agent 只读。

容器使用 [`examples/control-agent/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/control-agent/compose.yaml)。
sidecar 共享 HBBS network namespace 仅为了保持 `127.0.0.1:21115` 为本地通道；Agent
读写 Starry config volume，HBBS 以只读方式挂载同一 volume。示例 Agent 绑定 host
loopback，并默认只读。

## 只读接入

首次部署保持 `write_enabled: false`。Agent 仍正常认证请求，但只公布读取/模拟 capability，
对 plan、apply、rollback 与 runtime reload 返回 404。

按顺序验证：

1. 无 client cert、错误 CA、错误 URI SAN 均失败；
2. service JWT missing/expired/错误 audience、`azp` 或 scope 均失败；
3. `GET /control/v1/capabilities`、`/status`、`/relays`、`/config/schema`、`/config`
   返回结构化数据；`/config` 包含精确 managed UTF-8 YAML 与 strong ETag，但绝不读取
   secret-file reference 指向的内容；每个 Relay 条目同时返回最近一次 WSS 握手观测到的
   HBBR Starry 精确版本，legacy 或尚未探测的 endpoint 返回 `null`；
4. `POST /peers:verify` 使用 `starry.peer.verify` scope，只接受精确匹配的 peer ID、UUID、
   activation epoch/ID 与至少一个当前 route lease；仅在该 HBBS Profile activation 仍有效时
   返回 `registered: true`；
5. `POST /allocations:simulate` 返回 trace，重复调用不改变 rotation/health/generation 或
   production counter；
6. 公网无法访问 listener，HBBS `21115` 继续只在 loopback。

patch-v1.3.0 的能力发现包含 `relay_quality: 1`、`relay_active_probe: 1`、
`relay_probe_protocol: 1`、`relay_load_protocol: 1`、`fast_relay_authorization: 1`、
`profile_activation_lease: 1` 与 `peer_registry: 2`。
`/relays` 响应包含聚合的 `quality`、`fast_relay` 和 `profile_activation` 运行
对象。每个 Relay 返回显式 probe/load capability 版本、telemetry 观测时间/年龄/stale
状态及当前是否为质量候选；质量计数聚合 accepted、late、invalid 与 binding mismatch，
以及当前 `adaptive`/`eager` strategy、primary probe/accept、expansion、P2P cancel、估算
节省尝试数、expanded decision/timeout、hysteresis/cache hit、duplicate 和 stage mismatch；
绝不暴露客户端完整 IP、allocation/session UUID、stage token、nonce 或原始报告。极速 Relay 计数用于
区分签发、完全重试复用、送达和 fail-closed 原因，绝不包含
令牌、会话 UUID 或签名授权。Kessoku 应先检查 capability 与 schema v4 digest，再通过与
其他配置相同的 validate/plan/apply/audit 事务提供 FastCompat 控制。

Profile activation 计数区分 lease/ACK/续期/注销/清理、stale、rate、TTL 与有界容量结果，
绝不包含 peer ID、UUID、public key、activation ID 或 route lease。Kessoku 应另外以
`profile_activation_lease: 1` 为 Profile 切换门禁，按 HBBS `instance.id` 保存 lease，并对
每个签发实例调用 `/peers:verify`；lease 不是集群级凭证。详见
[Profile Activation Lease v1](../../reference/PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)。

每个响应包含 `X-Request-ID`；有效 W3C `traceparent` 会进入 mutation 的持久 audit。不得把
用户 raw connection token 或极速 Relay 签名授权发送给该 API。

## 开启配置写入

只有在目标 filesystem 上完成 staging apply/rollback 与 outage recovery 后才设置
`write_enabled: true`。正常变更流程：

1. `GET /config`，保留对磁盘精确 bytes 计算的 strong ETag；
2. `POST /config:validate` 提交 YAML candidate；
3. `POST /config:plan` 携带 `If-Match`，检查 risk、changes、digest、instance、generation
   与 expiry；
4. `POST /config:apply` 携带同一 `If-Match`、candidate digest、plan ID 与唯一 16–128 byte
   `Idempotency-Key`；
5. 轮询 `GET /operations/{id}` 至 `succeeded`，再将 activation ack 与 `GET /config`、
   `/status` 比较。

所有位于 `/relay_quality` 或其子路径的 JSON Pointer 变更至少分类为 `medium`，包括
启用/关闭功能、修改 strategy 或阈值，以及整体替换/删除对象。鉴权模式等更高风险变更
继续保持既有 `high`/`critical` 优先级。

Agent 拒绝并发/stale plan 与外部 disk drift；idempotency key 只能重放完全相同的 mutation。
只有原子替换磁盘文件且 HBBS 确认 source digest、effective digest、generation 与所有必需
subsystem 后才报告 apply 成功。

terminal operation、idempotency、audit 与 recovery record 在 24 小时后过期，并额外受数量
和 256 MiB 总 state budget 限制。pending/running/manual-intervention record 绝不会自动清理；
受保护记录填满 store 时会 fail closed，必须由 operator 先完成 reconciliation。

rollback 是从 `/config/history` 选择的全新审计事务，不删除历史。`restart_required` plan
不会被 apply，Agent 也绝不调用 Docker 或 systemd。

## 恢复 runbook

operation 进入 `rolled_back`、`failed` 或 `manual_intervention_required`、runtime/disk drift、
以及 Agent audit/state 持久化错误都应告警。

普通自动 rollback 后，先确认 disk ETag 与 HBBS source digest 已恢复到操作前值，再用新
plan 和 idempotency key 重试。遇到 `manual_intervention_required`：

1. disable/stop Agent 或切回只读；
2. 保留 `config-history/operations`、`audit`、`recovery`、`revisions`、`idempotency` 作为
   incident evidence；
3. 对比 managed file 精确 bytes/owner/mode、local HBBS runtime generation/digest 与
   operation recovery manifest；
4. 恢复经过审核的 last-known-good bytes，并执行本机 acknowledged reload；
5. 只有证明 disk/runtime 一致后才重启 Agent。

绝不能为了清除阻断而直接删除 state directory；这会丢失判断哪些 bytes/runtime 曾活动所需
的证据。
