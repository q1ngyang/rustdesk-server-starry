# patch-v1.3.2 preview 版本说明

[English](RELEASE-NOTES-patch-v1.3.2.md) | **简体中文**

patch-v1.3.2 精确基于已发布 tag `1.1.16-patch-v1.3.1`（commit
`1b8080bf074e3236cf9a3c0dfae2bdf16832249e`）开发，是向后兼容的活动会话
续期补丁，只修改 Starry。Relay Quality v1、schema v5、AKR1 kind 1–5、可靠 HBBR
数据面、官方客户端行为以及 patch-v1.3.1 的字段 1–12 均保持冻结。

**发布状态：仓库内闸门通过后为 PREVIEW_APPROVED。** 只允许 prerelease 与滚动
`preview` 镜像，不允许 `stable` 或 `latest`。后两者仍需真实 Akari controller/target
长会话、跨网 NAT/UDP 故障 soak 和不可变托管产物 provenance。

## 已冻结续期契约

协议先于实现冻结在 commit
`1844654a272d70112bbbc7774414320c98aa3b99` 与
`f65981fbf8c77dd93cfd026422e528128aa13d1c`。规范 manifest 为
[`contracts/patch-v1.3.2/CONTRACT-RELEASE-SUMMARY.json`](../../contracts/patch-v1.3.2/CONTRACT-RELEASE-SUMMARY.json)，
SHA-256 是
`0158980c7c9b3e3d50cda29d5737fefc52a3324da492034542f91d8ac5c55784`。
其中逐文件哈希冻结：

- Rendezvous tag 106/107 与 Fast Relay 授权字段 13–16；
- 续期 lifecycle、单调性、绑定、replay、admission、隐私和兼容规则及语义 fixture；
- 认证 Relay telemetry v3 schema/fixture；
- Control OpenAPI 1.1 与 typed `capabilities`/`relays` fixtures。

schema v5 没有改变，其唯一机器表达仍为 `capabilities.config_schema = 5`。
续期独立协商为 `capabilities.fast_media_relay_renewal = 1`；单 Relay 表达为
`relays[].capabilities.fast_media_relay_renewal = 1`。仅在 HBBS 收到新鲜、认证的
telemetry v3 且 `fast_media.renewal_protocol = 1` 时成立，禁止根据版本字符串推断。

## 实现变化

- HBBS 为自己最终选定的 Relay 保存有界、分角色授权链；只用现有 Starry Ed25519
  密钥签发 controller/target 两份新授权，并仅从承载请求的认证加密信令 route 返回。
  客户端和 HBBR 均不能自签或续签。
- WSS 继续精确绑定原 route。原生初始 Relay 响应会消费一次性 writer，因此后续
  Secure TCP 认证请求只允许 controller 源端口变化；源 IP、会话、Relay、allocation、
  协议、cap、sequence 和授权 digest 必须完全相同。明文 TCP 或 IP 改变 fail closed。
- 完全相同重试返回逐字节相同 response；冲突 request ID、旧/跳跃 sequence、会话 ID
  改变、角色 hash 不符、bitrate 升高、Relay/协议/datagram 改变及已过期新 grant 均拒绝。
- HBBR 只通过新的 AKR1 Cookie/Bind 安装续期授权。双角色 sequence 只可在有界过渡期
  相差 1；renewal/rebind 保留 2048 包 AKF1 replay window 和速率历史。HBBR 重启后可用
  尚未过期的新授权恢复同一会话。
- fully-bound 可续期 allocation 不再因 bootstrap 到期或旧的创建 300 秒上限而删除；
  half-bind/idle TTL、有界清理、容量/准入、过期角色拒绝、有界恢复期以及默认 12 小时、
  最大 24 小时绝对会话寿命仍生效，不存在永久 allocation。
- HBBS/HBBR 在接受角色前预留 wire-rate。签名的编码源 cap 只能降低，wire allowance
  保持 `ceil(source × 1.45)`；同 NAT 双角色累加到同一个有界 IP budget。
- 续期、UDP、grant、bind、admission、重启或限流失败只影响 FastMedia；可靠桌面/HBBR
  流保持连接，Akari 可回退并在同一 FastMedia session 自动重入。

## Akari 精确实现清单

以规范 protobuf
[`contracts/fast-media-renewal/v1/rendezvous-extension.proto`](../../contracts/fast-media-renewal/v1/rendezvous-extension.proto)
及状态机
[`FAST-MEDIA-RENEWAL-v1.zh-CN.md`](../reference/FAST-MEDIA-RENEWAL-v1.zh-CN.md)
为准。

1. FastMedia 全程保留可靠 Relay 会话。只有双角色 bootstrap grant 的字段 13
   `fast_media_relay_renewal = 1` 时启用续期，禁止从版本号推断。
2. 解析字段 14 `relay_session_id`、15 `renewal_sequence`、16
   `previous_authorization_sha256`；bootstrap 对应 session ID/sequence 0 与空前序 hash。
3. controller 选择非零 AKR1 session ID，通过既有已认证、端到端加密的可靠会话交给
   target；双方必须绑定同一值。
4. 到 `renew_after` 时，controller 在认证 WSS route 或同 IP 的新 Secure TCP 连接发送
   `FastMediaRenewalRequest`（oneof tag 106）：protocol 1、精确 UUID/allocation/session
   ID/current sequence、两份当前完整 combined grant 的 SHA-256、随机 16-byte
   request ID、既有 connection token、HBBS 选定 Relay/协议/datagram/bitrate，以及
   requester role 1。
5. 仅以相同 request ID 重试完全相同的逻辑请求，并使用有界退避。按数值处理 response
   tag 107 状态：`1 OK`、`2 DISABLED`、`3 UNAUTHENTICATED`、`4 NOT_FOUND`、
   `5 BINDING_MISMATCH`、`6 EXPIRED`、`7 TOO_EARLY`、`8 RATE_LIMITED`、
   `9 UNAVAILABLE`、`10 INVALID`。
6. 收到 `OK` 后校验全部 echo binding、sequence 为 `current + 1`、expiry 增加且 cap
   不升高；本地只安装 controller grant，只经现有认证加密桌面通道交付 target grant，
   不通过 Kessoku、telemetry 或 Control API。
7. 两个角色分别取得新 source-bound cookie，并用新 grant 发送 AKR1 Bind。只在有界
   transition 内容忍 sequence 差 1；renewal/源迁移不得重置 AKF1 sequence/replay 状态。
8. 若到 `fallback_before`（到期前 10 秒）双角色新 Bind 仍不可用，最迟此时切到可靠
   媒体；保留末帧、输入/控制与可靠 session。有界重试后可用相同 session ID 自动重入。
9. 客户端不得自选 Relay、自签、接受过期 grant、提高 cap、改变固定 binding，亦不得
   把 allocation ID 单独当作权限。

Akari 验收至少覆盖 controller/target 完全重试、冲突 replay、第二角色延迟安装、源端口
迁移、listener 重启、可靠回退与同 session 自动重入，否则不得宣称实现完成。

## 兼容矩阵

| 部署组合 | 行为 |
| --- | --- |
| 官方客户端或官方/legacy HBBR | P2P/LAN/可靠 Relay 不变，未知增量被忽略。 |
| 旧 Akari + patch-v1.3.2 | 使用 bootstrap grant，在原 expiry 安全回退。 |
| 新 Akari + patch-v1.3.1 HBBR | 无认证续期能力，保留 v1.3.1 的 90/300 秒与回退行为。 |
| patch-v1.3.2 HBBR + 旧 HBBS | HBBS 不公布续期能力，仍按 bootstrap 行为运行。 |
| 新 Akari + patch-v1.3.2 HBBS/HBBR | 仅在服务端最终 Relay、auth allow 与匹配的新鲜 telemetry v3 均成立时续期。 |
| 官方/新 Akari 混合 | 官方端忽略私有消息，服务端选定的可靠 Relay 始终权威。 |

所有 schema-v5 Fast 开关仍相互独立且默认关闭。telemetry-v2 Relay 仍可参与 v1.3.1
bootstrap FastMedia，但不可续期。

## Telemetry、Control 与隐私

认证 telemetry v3 增加有界 renewal、replay、reservation、admission、transition/
recovery、剩余 TTL 与临近到期聚合；Control API 1.1 只暴露脱敏固定字段。两条路径均不
包含完整客户端地址、session/allocation/request ID、nonce、token、stage token、grant、
原始报告或媒体内容。

`process_instance_id` 只在 Starry 内用于识别 HBBR 重启。Kessoku 必须在入口丢弃，
不得透传、持久化、索引、记录日志或显示。

适合告警：持续 stale/unhealthy telemetry、启用但不健康的 UDP listener、持续增加的
listener/auth/admission/rate/expired 拒绝、续期临近到期、最小剩余 TTL 过低、reservation
逼近上限。bind/rebind/renewal 成功数、replay 分类、packet/byte 与角色过渡只适合诊断，
除非与可靠回退或用户故障相关。

## Kessoku 结论

Kessoku 不在 grant 交付、签名、媒体或续期链路。v3.0.8 可安全忽略新增 Control 字段并
继续管理既有 schema-v5 配置，但看不到续期聚合。仅当需要读取/展示新的 typed capability
和聚合 telemetry 时才需要 Kessoku v3.0.9。它必须 pin 干净、已推送的 contract/source
候选，不能 pin dirty worktree，并必须执行上述 `process_instance_id` 丢弃规则。

## 升级与回滚

顺序为 HBBR、HBBS、Control Agent、最后是支持续期的 Akari。认证 telemetry v3 新鲜且
普通 Native/WSS/mixed Relay 回归通过前保持 FastMedia 关闭；先对有界 Relay/客户端组
canary。

schema v5 未变，因此回滚无需 schema 转换。先停止签发续期，让客户端回到可靠流；用
activation ACK 关闭 FastMedia 并等待 allocation/grant drain，然后先把 HBBS、再把 HBBR
回滚到 `1.1.16-patch-v1.3.1`。旧二进制忽略字段 13–16 和 telemetry-v3 增量；必须保留
enrollment、identity、YAML/PEM/JWKS、普通 Relay/WSS 与配对状态。再次升级后需重新取得
新鲜 telemetry v3，才可恢复续期。

## 验证与剩余闸门

经评审实现候选为 `e44f6a0380914454cc543ebb6cdb031f5b3e08f9`；机器可读
[`验证 manifest`](VERIFICATION-patch-v1.3.2.json)记录精确命令、计数、基线 revision 与
apply-twice 哈希。源码候选包含 unit、contract、可控时钟与真实 HBBS/HBBR 子进程覆盖：
双角色 bootstrap/
renewal、完全重试、冲突/过期/binding/cap 拒绝、角色延迟、replay 保留、同 IP native
源端口变化、精确 WSS route、明文拒绝、HBBR 重启恢复、AKF1 转发、admission/rate limit
及可靠流存活；并保留 protocol、Control、mixed Relay、连接认证、local control、
WebSocket 与 1000 注册发布闸门。精确命令结果记录在评审 source candidate 的验证
manifest 中，发布 CI 会再次执行；该纯源码证据不宣称已有不可变 tag 或托管 image digest。

以下只阻止 `stable`/`latest`，不阻止源码可控的 preview：

- 带依赖锁与 provenance 的不可变真实 Akari controller/target 构建，完成正常时长续期桌面会话；
- 可追溯设备/runner 上的真实 NAT、UDP blocked、AP/源迁移、listener restart、突发丢包、
  持续超限、回退与自动重入；
- 同一精确评审 commit 的不可变 tag、source commit、image-index/linux-amd64 digest 与
  托管发布证据。
