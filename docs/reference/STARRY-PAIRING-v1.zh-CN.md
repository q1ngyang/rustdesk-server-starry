# Starry Pairing v1

[English](STARRY-PAIRING-v1.md) | **简体中文**

Starry Pairing v1（`SP1`）使用短期、单次代码引导 Control Agent 或单个 Relay。
它不是长期控制协议。领取完成后，Control Agent 继续使用现有 TLS 1.3 mTLS 和有 scope
的 service JWT；每个 Relay 继续使用节点身份、独立 telemetry secret、签名 Fast Relay
授权和普通 HBBR 数据面。已经建立的 RustDesk 会话不依赖 Broker、Kessoku 或 Control
Agent 持续在线。

规范文档由
[`contracts/starry-pairing/v1/pairing.schema.json`](../../contracts/starry-pairing/v1/pairing.schema.json)
定义。

## SP1 代码与领取绑定

代码格式为 `SP1.<base64url-no-pad canonical JSON>`，包含：

- 协议版本 1，以及用途 `control-agent` 或 `relay`；
- 精确 HTTPS Broker origin 和 SHA-256 SPKI pin；
- enrollment UUID、已批准配置 digest、过期时间和 256-bit 随机 secret。

客户端只从交互式 stdin 或 mode-0600 普通文件读取代码；命令行/环境变量代码、明文
HTTP、origin 漂移、pin 不匹配、未知字段、错误用途、过期和超大输入均被拒绝。claim
绑定 enrollment、用途、配置 digest、secret、request digest，以及本地生成 key/CSR
fingerprint。

第一个有效 claim 原子绑定该 key。响应丢失时，只有 Broker 恢复窗口内完全相同的
request/key 才能取回相同结果。其他 key、CSR 改变、用途交换、digest 漂移、撤销/过期
后重放或 endpoint 漂移均 fail closed。原始代码、私钥、telemetry secret、CSR 内容和
完整证书链不会进入日志或指标。

写入任何身份前，客户端会验证带 pin 的 Broker TLS、返回的 enrollment/request 绑定、
key 与证书匹配、CA 签名和证书有效期。

## Control Agent 配对

Control Agent 在本机生成服务端私钥和 CSR。Broker 返回服务端证书、客户端 CA、允许的
client URI SAN、service JWKS、JWT issuer/audience、精确 Agent origin、instance UUID 和
中心公钥。生成的运行 YAML 只使用 patch-v1.3.0 已经支持的 Control Agent v1 字段。

```console
starry-control-agent pair
Paste pairing code:
```

`pair` 要求目的目录为空，不覆盖已有身份、instance UUID、本地 token 或 YAML。
`adopt` 显式接管已有 Starry 实例，同时保留实例身份；`rotate` 只通过新的绑定 SP1 claim
轮换托管证书材料。所有模式均支持显式 `--state-dir`、`--identity-dir`、`--output`、
`--shared-dir`、`--managed-config`、`--backup-dir`、`--listen`、
`--local-control-address` 和可选 `--broker-ca-file`。Agent 证书通过 DNS 名使用时，必须用
`--tls-server-name`（或 `STARRY_CONTROL_AGENT_TLS_SERVER_NAME`）传入 allowlist 中完全
相同的名字；该名称作为 DNS SAN 写入本地生成的 CSR。IP literal、端口、URL 语法、空
label 和非法 DNS label 会在本地拒绝。

配对具备崩溃幂等性：重试时完全相同的已安装文件可接受，任一不同字节都会拒绝而不
覆盖。首次 `pair` 中断后还会恢复 durable pending record 已绑定的 instance UUID，不会
静默生成第二个身份。`rotate` 校验所有生成 Agent-v1 path binding 后，保留现有 Agent
listen address、local-control address、managed-config 大小上限和写策略；Broker 返回的
trust material 仍只由新绑定响应替换。生成的 local control token 只在 Agent 与 HBBS
之间共享，不返回 Broker。

## Relay enrollment

Kessoku 可以代理 SP1 claim，但经认证的 Starry Control Agent 才是已批准 Relay endpoint、
池、profile、容量、draining 状态和配置 digest 的授权方。Control API 在
`/control/v1/relay-enrollments` 下提供有界 `prepare`、`complete`、健康门控 `activate`、
`revoke`、list 和 get；
写操作需要 `starry.relay.enroll`、Agent write-enabled 策略、mTLS、service JWT 与普通
幂等控制。除一次必要 claim response 外，API 响应不暴露 SP1 secret、私钥或已存储的
telemetry secret。完全相同 key/CSR 的重试只可在十分钟有界窗口内恢复丢失的领取响应；
窗口结束后，即使请求完全相同也不能再次取得含 secret 的 bundle。

Relay 在本机生成节点 key，并使用独立工具，完全不改变上游 `hbbr` CLI：

```console
starry-relayctl enroll --data-dir /var/lib/rustdesk-server-starry/relay
Paste pairing code:
```

安装后的 `starry/enrollment` 包含节点身份与证书、Relay CA、中心公钥、每 Relay 独立
telemetry secret、已批准运行 JSON、host binding 和 `relay-compat.env`。兼容文件只含
公钥、非 secret 限制/endpoint 和 telemetry secret 文件路径，绝不含 secret 值；
`starry-relay-entrypoint` 使用固定 key allowlist 解析它。
非 secret 的完成 marker 还保留 enrollment ID 与已批准 configuration digest。如果进程
在 marker 已持久化、但 pending 文件尚未清理或成功响应尚未返回时停止，完全相同的重试会
先校验已安装 key/certificate 及这些绑定，只删除匹配的 pending 文件，再返回相同完成
状态；发生变化的 code、key、CSR、用途或 digest 绝不作为恢复接受。

撤销先持久化 registry 状态，再清理当前 Relay 的 certificate、telemetry secret 和 claim
marker。清理绑定精确 claim，把 marker 留到最后，并可在部分清理中断后重试。相同 node
的后续 enrollment 只可替换 registry 中匹配记录已为 `revoked` 或 `expired` 的 claim；
active、未知、不匹配或并发变化的 claim 一律 fail closed。兼容文件解析器还会保留公开
RustDesk `KEY` 的标准 Base64 padding，不会把末尾 `=` 错当成分隔符丢弃。

Relay-only Compose 在 enrollment 前即默认 `RELAY_REQUIRE_ENROLLMENT=1`（映射为
`STARRY_REQUIRE_RELAY_ENROLLMENT=1`）。先让 `starry-relayctl` 以一次性命令使用同一个
bind mount，再启动 HBBR。这个持久运维意图使空目录、被删除或错误挂载的
`RELAY_DATA_DIR` 在 HBBR 静默退化成全新手工 Relay 前直接失败。手工/官方 HBBR 仍受
支持，但必须由运维显式把该值设为 `0`；已 enrollment 的 Relay 不得复用这个退路。

已 enrollment Relay 不能自行加入 GEO 池或更改批准的公网 endpoint。
`activate_after_health` 是不可变高风险预授权；没有它时 enrollment 保持 pending，等待
现有 Agent 配置事务。预授权领取后先进入 `claimed_pending_health`。正常
`validate -> plan -> apply` 事务成功后，调用方只向 `relay-enrollments:activate` 提交该
operation ID、生成的 config generation 与 health snapshot ID；Agent 校验持久 activation
ACK，并独立重读 HBBS inventory。只有精确批准的 Relay、Native 健康、版本，以及 profile
要求的 fresh 认证 schema-2 telemetry、WSS 健康、capacity/draining 和 FastMedia UDP
capability/端口/健康全部匹配，才原子进入 `active`。相同证据重试幂等；旧 generation、
endpoint 漂移、不完整 ACK 或证据变化均拒绝。DNS、证书、防火墙及外部观测的 Native、
WSS、telemetry、UDP 健康仍是部署前提。

## 持久化布局

容器状态必须位于显式挂载根：

```text
STARRY_PERSIST_ROOT/
  control/state/       instance 与有界 enrollment registry
  control/identity/    Agent identity、client CA 与 Relay CA
  control/generated/   Agent v1 运行 YAML
  control/shared/      HBBS/Agent 本地 token
  relay-secrets/<id>/  每 Relay 材料
  config/              当前配置、快照与历史
  hbbs/                RustDesk identity 与 HBBS 状态

RELAY_DATA_DIR/
  starry/enrollment/   Relay identity、配置与兼容导出
```

配对和服务启动会拒绝相对路径、要求普通文件/目录处的 symlink、不安全 owner/mode、
overlay/tmpfs 容器层、缺失显式 mount 及 Relay host identity 不匹配。同一 Relay enrollment
目录不能在两台主机同时在线。只有继续挂载相同显式宿主根时，`docker pull`、
`force-recreate`、`down`/`up` 才保持身份；`down -v`、更换相对宿主路径或把同一身份复制
到并发主机都是操作错误，必须在启动前失败。

原生/DEB 默认分离配置与状态：

```text
/etc/rustdesk-server-starry/
/var/lib/rustdesk-server-starry/
```

普通 package upgrade/downgrade 不覆盖身份；删除身份必须是正常升级路径之外的显式
管理员 purge。

## 手工兼容与降级

现有手工 Agent YAML/mTLS/JWT 和手工 HBBR 公钥/telemetry secret-file 部署继续支持，
pairing 完全可选。patch-v1.3.0 能读取生成的 Agent v1 YAML、PEM 和 JWKS。已 enrollment
HBBR 临时运行 patch-v1.3.0 时可使用 `relay-compat.env`，保留普通 Relay 与 telemetry，
并忽略 enrollment 和 FastMedia 状态。

v1.3.1→v1.3.0 回滚前，证书剩余有效期必须至少九十天。先关闭并 drain FastMedia、导出
schema v4，再依次替换 HBBR、HBBS 和 Agent。旧二进制必须忽略而不能删除保留的
SP1/enrollment 状态；重新升级 v1.3.1 后复用同一身份，并在再次启用 FastMedia 前重新
通过健康检查。
