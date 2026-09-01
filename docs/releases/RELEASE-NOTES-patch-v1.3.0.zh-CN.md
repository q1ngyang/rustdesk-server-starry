# patch-v1.3.0 版本说明

[English](RELEASE-NOTES-patch-v1.3.0.md) | **简体中文**

patch-v1.3.0 为 Akari 增加三个重点功能，同时保留官方 RustDesk 客户端原有 wire path：
候选 Relay 主动探测与质量评分、面向 Akari P2P 极速模式的签名 `FastCompat` Relay 授权，
以及带有界快速重新注册的 generation-safe Profile Activation Lease。经审核的发布准备变更
已记录批准；只有将要获得不可变 tag 的精确 commit 再次通过源码、安全、协议、包、镜像和
发布候选全部门禁后，工作流才允许发布。

## 主要变化

HBBS 先按传输方式、健康状态、GEO 规则和配置顺序筛选，最多保留五个 Relay 候选。默认
`adaptive` 策略先只让两端 Akari 探测 GEO primary；HBBS 用服务端阈值判断分数和丢包是否
good-enough，达标即选 primary，不达标才同时让双方扩展探测其余候选。显式
`strategy: eager` 保留一次探测全部候选的模式。只有 HBBS 能把报告与可信 HBBR load
结合、选择最终 Relay，并把同一结果写入普通 `relay_server` 和 decision 扩展。

每个阶段使用全新 token 和服务端 deadline。report 同时绑定 allocation、stage、token、
端点角色、配置 generation、精确信令 route 与双方 IP；重复报告幂等，旧阶段或重放报告
不能改变决定，有界 HBBS timer 在 deadline 形成 partial 或 fallback。P2P 成功会显式取消
allocation，不再等待 Relay 探测。Force Relay、对称 NAT、WSS 与 mixed 路径都必须等最终
服务端 decision 后才能连接 HBBR。

默认综合分范围为 `0..10000`，分数越高越好：

- RTT 占 40%；双端均成功时，有效 RTT 为
  `(2 × max(RTT_A, RTT_B) + RTT_A + RTT_B) / 4`；
- jitter 占 20%，取两端较差值；
- loss 占 25%，取两端较差值；
- Relay load 占 15%，只采用 HBBS 自己观测的 HBBR 数据。

缺失端点报告会受到可配置惩罚。对称的 IPv4 `/24` 或 IPv6 `/56` 网段缓存和分数迟滞
用于减少 Relay 抖动；缓存键不保存完整客户端 IP。

HBBR 会在原生 framed TCP 或 WebSocket Relay 连接上接收一次有界
`RelayProbeRequest`，并返回绑定 nonce 的 `RelayProbeResponse`。公开 `/ws/relay` 握手和
probe response 只给出精确 Starry 版本及 probe/load 协议能力，不包含详细 load。HBBS
只能通过受认证 `/ws/telemetry` 取得 active sessions、pending pairs、带宽 EMA、容量、
draining/admission 状态及聚合计数。上游会话配对与字节转发循环保持不变。

HBBR 对客户端 probe 同时执行全局和源 IP 有界速率限制，只发布 malformed、unsupported、
rate-limited 与 successful 聚合计数。`STARRY_RELAY_MAX_SESSIONS` 现在是真实 admission
上限：达到容量或 draining 时只拒绝新配对，不终止已有会话。每个签名 schema-v1 snapshot
携带进程实例 ID、单调 sequence、观测时间、uptime 与 admission rejection，HBBS 对重放、
过期数据和实例重启按 fail-closed 处理。

混合会话在同一 allocation 下接收端点定制的探测 URL：原生端测原生 TCP，WebSocket
端测 WSS。

连接鉴权和最终 Relay 质量决策完成后，HBBS 可使用现有 Ed25519 密钥签发短期
`FastRelayAuthorization`。目标端 `RequestRelay` 与控制端 `RelayResponse` 会收到
完全相同的签名字节。服务端内存把授权与会话 UUID、发起端 IP、目标端 IP、最终
Relay、allocation 及当前配置 generation 绑定；合法重试复用原决定与签名，不延长
有效期。

本版只授权基于现有可靠 HBBR 数据流的 `FastCompat`。每个签名授权均明确设置
`allow_fast_media_v1 = false`；patch-v1.3.0 不实现、不广播 Relay 侧 FastMedia UDP。

Akari 注册现在可以携带 16 字节随机 activation ID 与单调递增 epoch。成功 Ready ACK 会
原样返回二者，并返回 32 字节不透明、节点本地 route lease 与 HBBS route generation。
只有所有值都匹配后 Akari 才提交 Profile 切换。Native UDP/TCP 与 WSS 共用同一 generation
authority；显式 `DeactivatePeer`、WSS reader 退出、idle drain、延迟包及 A→B→A 切换都
只能移除精确当前 route。45 秒 lease TTL 继续作为崩溃兜底。

已验证快速重新注册要求已存 peer ID、network identity UUID 和 public key 完全一致；全局
/IP blocker 仍启用，同时在滚动 30 秒内最多新签发 12 个 lease。该优化既没有删除注册限流，
也不依赖缩短 TTL。HBBR 不新增 Profile activation 数据消息。

## 兼容约定

所有 Starry 字段均为 protobuf 增量扩展：Relay 质量消息使用 tag `100+`，不透明签名
授权在 `RequestRelay` 与 `RelayResponse` 使用未占用的 tag `64`。没有改号或改变任何
官方字段类型：

| 发起端 | 被控端 | 行为 |
| --- | --- | --- |
| 官方客户端 | 官方或 Akari | 传统 HBBS 选择，只返回普通 `relay_server`，不会创建质量分配或 FastCompat 授权。 |
| Akari 协议 v1 | 官方客户端 | Akari 先探测 primary，必要时单端探测其余候选；缺失被控端报告会受惩罚并标记 `partial`。官方端仍使用最终普通 `relay_server` 并忽略 tag 64。 |
| Akari 协议 v1 | Akari 协议 v1 | 双方先探测 primary，仅在需要时同步扩展，并收到同一个 HBBS 决定；全部授权门禁通过时双方收到同一签名 FastCompat 授权。 |
| Akari 未取得 offer | 任意 | 直接 `RequestRelay` 继续走传统路径；强制 Relay 模式应先做支持质量协议的 PunchHole 预检。 |

官方 HBBR 不认识探测 oneof，会直接关闭这条短探测连接。它只能作为显式配置的普通
`relay_server` fallback，不得进入质量 offer 或占用 `max_candidates`。生产质量池必须使用
明确声明 `relay_probe_protocol=1` 与 `relay_load_protocol=1` 的 HBBR；版本字符串只用于
inventory，绝不表示能力。

规范字段绑定和消息定义见
[Relay Quality v1 合约](../reference/RELAY-QUALITY-PROTOCOL-v1.zh-CN.md)与
[极速 Relay 授权协议 v1](../reference/FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)。匹配 ACK、
注销、多节点、发布和回滚规则见
[Profile Activation Lease v1](../reference/PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)。

## 配置

Relay 质量功能要求 schema v4，默认关闭：

```yaml
version: 4
relay_servers:
  - relay-asia-1.example.com:21117
  - relay-asia-2.example.com:21117

websocket_signal:
  relay_health:
    endpoints:
      - relay: relay-asia-1.example.com:21117
        url: wss://relay-asia-1.internal.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry
      - relay: relay-asia-2.example.com:21117
        url: wss://relay-asia-2.internal.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry

relay_quality:
  enabled: true
  strategy: adaptive
  legacy_fallback_relays: []
  max_candidates: 3
  primary_probe_samples: 3
  primary_accept_score: 8000
  primary_max_loss_basis_points: 500
  p2p_probe_grace_ms: 300
  probe_samples: 5
  probe_interval_ms: 50
  probe_timeout_ms: 1000
  report_timeout_ms: 15000
  max_telemetry_age_seconds: 180
  allocation_ttl_seconds: 30
  cache_ttl_seconds: 300
  max_allocations: 10000
  hysteresis_basis_points: 500
  missing_report_penalty_basis_points: 1000
  rtt_bad_ms: 300
  jitter_bad_ms: 100
  weights: {rtt: 4000, jitter: 2000, loss: 2500, load: 1500}

fast_mode:
  relay:
    fast_compat_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
```

四个权重必须均大于 0 且合计 10000。启用时至少配置两个非 legacy Relay，每个质量
Relay 都必须有唯一 telemetry endpoint，且 `max_candidates >= 2`。每个 HBBR 都应配置
符合实际容量的会话上限，例如：

```sh
STARRY_RELAY_MAX_SESSIONS=10000
```

`TOTAL_BANDWIDTH` 仍是上游 HBBR 使用的 Mbit/s 容量设置。HBBS 只使用自己通过
证书校验和请求认证取得的 HBBR snapshot。YAML 只保存 secret-file 绝对路径，
不保存 secret 值；内部 mTLS 仍是首选，HMAC 用于反向代理终止 TLS 的部署。缺失、
legacy、不完整、重放或过期遥测会让 Relay 退出质量 offer，绝不信任客户端提交的
load 参与选择。`legacy_fallback_relays` 是保留普通旧版 fallback 的显式入口。为质量采集
配置的 Relay health 探测在 HBBS WebSocket Signal 关闭时仍可运行，但不会因此使
WSS/mixed 分配变为 eligible。详见
[Relay Telemetry v1 合约](../reference/RELAY-TELEMETRY-v1.zh-CN.md)与
[安全/运维指南](../wiki/ZH-CN-Relay-Telemetry-Operations.md)。

FastCompat 独立默认关闭。启用它必须先启用 Relay 质量，把连接鉴权设置为 `audit` 或
`enforce`，并启用 Secure TCP 或 WebSocket 信令；在 audit 模式中，有效令牌仍须得到
严格的 `allow` 才能签发。TTL 范围为 `30..300` 秒，码率范围为
`1000..200000` Kbit/s。若反向代理终止 WSS，必须阻止公网直接访问 HBBS 明文
WebSocket 监听端口。

## Kessoku 与 Control API

Control API v1 保持最小权限边界，并声明 `relay_quality: 1`、
`relay_active_probe: 1`、`relay_probe_protocol: 1`、`relay_load_protocol: 1` 与
`relay_telemetry_schema: 1` 与 `fast_relay_authorization: 1`。
同时声明 `profile_activation_lease: 1`。
`GET /control/v1/relays` 返回每个 HBBR 的能力版本、schema/实例/sequence/uptime/重启状态、
active/pending/带宽/容量、draining/admission 计数、telemetry 观测时间/年龄/stale 状态、
accepted/late/invalid/binding mismatch 报告计数、固定 offer/fallback reason 及有界的每 Relay
选择计数、当前 strategy、primary probe/accept、expansion、P2P cancel、估算节省尝试数、
expanded decision/timeout，以及有界的
极速 Relay 签发、复用、送达和 fail-closed 原因计数；`profile_activation` 聚合 lease、
ACK、续期、清理、stale、rate、TTL 和容量计数而不暴露 peer/lease secret。Kessoku 可通过
`/peers:verify` 按 HBBS 实例核对精确当前 activation。

Kessoku 可以配置和观察这些状态。候选与探测报告只通过 HBBS 信令传递；Akari 不连接
Control Agent，Kessoku 也不代理 Relay 数据。schema/plan/apply/audit 不得暴露或持久化
HBBS 签名私钥、连接令牌、会话 UUID 或签名授权。

Control Agent 会把所有 `/relay_quality` 变更至少分类为 `medium`；已有更高分类的安全
敏感变更仍保持 `high` 或 `critical`。

## 升级与回滚

1. 质量功能保持关闭，先部署内部 TLS/mTLS 策略和只读 secret file；
2. 先升级 HBBR，确认公开 probe 不含详细 load，且每个质量 Relay 通过认证提供
   schema-v1 telemetry、显式 probe/load capability、稳定实例与递增 sequence；
3. 再升级 HBBS，仍使用 schema v3 或关闭质量，验证新鲜 telemetry 及隐私安全的本地
   Control inventory；
4. 最后升级 Control Agent；它只读 HBBS local control，不连接 HBBR。Kessoku 只在独立的
   Profile activation 功能中升级，不抓取 HBBR、不代理 telemetry；
5. 灰度 Akari 的匹配 ACK Profile 切换，覆盖 Native/WSS 和多 HBBS 节点，再通过
   validate/plan/apply 提交启用 Relay 质量的 schema v4；
6. 将连接鉴权从 audit 推进至 enforce，确认安全信令后，再通过独立计划变更启用
   `fast_mode.relay.fast_compat_enabled`；
7. 发布门禁中继续保留普通直连/P2P 与官方客户端验收。

回滚 patch-v1.2.0 前，必须恢复不含 `fast_mode` 和 `relay_quality` 的 schema v3 配置；
patch-v1.2.0 会拒绝 schema v4 及这两个功能字段。先关闭 FastCompat，等待超过授权 TTL，
再删除字段。质量决定和签名授权缓存只存在于进程内存，不需要迁移数据。

若只回滚 HBBR，先关闭 Relay Quality（或把目标节点移入 `legacy_fallback_relays`），等待
至少一个 `report_timeout_ms` 后再替换 HBBR，避免能力消失的节点仍留在 in-flight offer；
HBBS 最后回滚。

回滚 Profile activation 时，先停止新的 Akari 切换并保留最后已提交 Profile，向每个签发
HBBS best-effort 注销，再等待至少 45 秒并加上 WSS idle/drain。状态仅存在于进程内存，
无需数据库迁移；旧服务端不会返回匹配增强 ACK，因此合规 Akari 会保留原 Profile。

## 已包含的验证

- 旧认证 fixture 解码后所有质量字段均为 protobuf 默认值；
- offer/report/decision 与主动探测消息可通过生成的 protobuf 代码往返；
- 真实 HBBR 进程对外只公开 capability/版本，仅在认证后返回签名详细 telemetry；
  原生 TCP、WS 及 TLS 终止的 WSS probe 会回显 nonce、执行限速，并继续完成
  原生↔WebSocket 双向字节转发；
- 实进程生命周期测试覆盖 pending→active配对、容量 admission、draining、已有会话存活、
  telemetry sequence 和进程实例重启语义；
- 单元测试覆盖 GEO primary 达标时不扩展、任一端 primary 不佳触发扩展、双端相同最终
  decision、eager 模式、官方对端 partial、阶段重放/deadline、P2P cancel 与资源上限，以及
  双端 RTT/jitter/loss/load、legacy-first GEO 候选上限、遥测过期、有界 health
  并发、迟到报告、不可实现 deadline、reload 保留、schema v4 门禁和权重校验；
- 真实 HBBS/WSS 信令测试覆盖最终质量选择、服务端覆盖不可信 tag 64、Ed25519 验签、
  FastMedia=false、向两端发送相同授权及重试完全复用；
- 实进程测试覆盖 Native UDP/TCP、WSS 旧 reader 清理、同 activation WSS 重试、延迟跨传输
  心跳拒绝、乱序续期/注销、A→B→A、旧注册
  默认值、精确当前注销和两个 HBBS 节点的独立 lease；单元测试强制每 30 秒最多 12 次
  verified burst 及容量边界；
- overlay 连续套用两次，并把修改后的 protobuf、合约和 `PATCH_VERSION` 纳入摘要。
- 发布流程创建或核验绑定精确 release commit 的 annotated tag，绝不移动已有 tag；带
  checksum 的 `STARRY-RELEASE-SUMMARY.json` 将已发布 image index/linux-amd64 manifest
  与 Control OpenAPI、配置 schema/UI schema、冻结的 Relay Quality 协议及 Relay 遥测
  schema 摘要绑定，供 Kessoku 固定。
