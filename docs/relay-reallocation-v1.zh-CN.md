# Relay 重分配 v1

契约状态：FROZEN。运行时发布状态：IMPLEMENTATION_PENDING。Akari 与 Kessoku 可以依据不可变契约实现，但运行时集成闸门通过前不得宣称可发布。

Relay 重分配 v1 与已冻结 Relay Quality v1 相互独立。能力使用 typed `relay_reallocation=1`；Rendezvous oneof 仅新增 tag 108–116，RequestRelay tag 103 与 RelayResponse tag 102 分别声明两端支持。字段缺失或为 0 时保持当前会话，不触发切换。

请求只能通过已认证的 Secure TCP/WSS 信令路径发送。客户端只提交当前绑定、16 字节幂等 ID，以及可选的配置 node ID，不能提交 Relay 主机或 probe URL。HBBS 绑定 session UUID、主控/被控角色、当前 route、当前 Relay、会话代次、配置 generation 与短期 deadline。双方同时请求按“deadline 更早、controller、request ID 字典序更小”确定唯一胜者。

候选只来自 HBBS 当前健康快照，必须具有显式 node metadata、新鲜 health 和 `relay_probe_protocol=1`。官方 HBBR 只可作为普通降级 Relay。display_name、region、relay_server、probe URL 与允许别名全部来自配置并全局唯一；禁止域名推断、重定向和客户端自选主机。

状态机是一次有界手动重评估，然后执行 `prepare -> 双端 ready -> commit -> 双端 commit ACK -> drain 旧路径`。两端 ready 的新可靠路径 binding digest 必须一致。HBBS 只序列化一份 commit，其中普通 `relay_server` 唯一且相同，并把完全相同的字节发给两端。双 ACK 前旧可靠会话始终有效；拒绝、超时、配置变化、路由消失、digest 不一致或新路径失败均让双方回滚。

prepare 会 fence 旧 FastMedia renewal 链；HBBS 为最终 Relay 重新签发 controller/target 授权。不支持 UDP 时保留 FastCompat 或标准可靠 Relay。新 allocation/session/key/replay 域必须轮换，旧 grant 与迟到 renewal 不得跨代安装。AKR1 kind 1–5 和 Relay Quality v1 不变。

Control API 只暴露 typed 配置和有界聚合计数，不返回 session UUID、request/reallocation/allocation ID、完整地址、probe URL、原始报告、nonce、token、grant、容量、带宽或密钥。Kessoku 不进入仲裁、签名、探测或媒体路径。

运行时发布闸门：双端按需探测与评分；真实双 patched HBBR 的 native/WSS/mixed 切换；三个失败点的旧路径 drain/rollback；FastMedia renewal 竞态及旧 grant 重放；受控时钟十分钟会话；高并发资源上界；真实 NAT/UDP 黑洞 soak。这些闸门不阻止契约冻结，但 Akari、Kessoku 和 runtime integration 状态继续 BLOCKED，且不得发布 preview/stable/latest。
