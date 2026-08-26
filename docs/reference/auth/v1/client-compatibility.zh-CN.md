# RustDesk 1.4.9 主控端连接认证兼容性

[English](client-compatibility.md) | **简体中文**

对应客户端源码版本：`rustdesk/rustdesk@1.4.9`。

| 场景 | 客户端行为 | Starry v1 约定 |
| --- | --- | --- |
| 拒绝 `PunchHoleRequest` | `src/client.rs` 先检查非空的 `PunchHoleResponse.other_failure`，再处理既有失败枚举；该文本直接作为连接错误返回。 | 返回 `failure=OFFLINE` 和 `other_failure="connection authorization failed"`。 |
| 拒绝直接 `RequestRelay` | `src/client.rs` 在建立 HBBR 连接前检查并返回 `RelayResponse.refuse_reason`。 | 返回 `refuse_reason="connection authorization failed"`。 |
| 被控端注册 | `src/rendezvous_mediator.rs` 独立完成注册并接收转发的 `PunchHole` / `RequestRelay`，不需要主控用户的 JWT。 | 不要求被控端登录；只认证发起连接的 `PunchHoleRequest` 或直接 `RequestRelay`。 |
| UDP `PunchHoleRequest` | 基于锁定的上游 1.1.16 版本，Starry 不处理 UDP `PunchHoleRequest`。受支持的连接发起请求通过原生 TCP、Secure TCP 或 `/ws/id` 到达 HBBS。 | 明确保持不支持 UDP 发起；不得查询目标、记录打洞请求或分配中继。 |

既有 protobuf 字段已足够，因此本约定不修改 `rendezvous.proto`，也不增加失败枚举。
面向客户端的错误文本保持稳定，不透露目标是否存在或内部令牌校验失败原因。

完整校验规则见[连接认证约定](profile.zh-CN.md)。
