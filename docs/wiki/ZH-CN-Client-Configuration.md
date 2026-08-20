# 客户端配置

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Client-Configuration) | **简体中文**

两端客户端必须配置同一个中心身份。真实客户端完成注册并传输桌面会话前，服务端部署不算完成。

## 必填字段

打开 RustDesk 客户端**设置 → 网络**，解锁设置后填写：

| 客户端字段 | 值 |
| --- | --- |
| ID Server | 公网 HBBS 域名，例如 `id.example.com`；非默认端口时显式写 `:21116`。 |
| Key | 中心 `id_ed25519.pub` 的完整单行内容；不是 API Token 或许可证 Key。 |
| Relay Server | 由 Starry HBBS 动态分配 Geo Relay 时保持为空。 |
| API Server | 只在部署独立 API 时填写，例如 `https://api.example.com`。 |
| 使用 WebSocket | 默认关闭；仅在该客户端网络需要 WSS 时逐端开启。 |

[官方 RustDesk 客户端配置说明](https://rustdesk.com/docs/en/self-host/client-configuration/)
同样区分 ID、Relay、API 与公钥字段；Starry 不改变其含义。

原生客户端需要同时可达 `21116/TCP` 和 `21116/UDP`。官方 1.1.16 服务端的被控端注册与
心跳走 UDP，控制端发起远控仍走 TCP/Secure TCP；`disable-udp` 不会把被控端注册改成
TCP-only。必须禁用 UDP 的被控端应启用 WSS，并完整部署 `/ws/id` 与 `/ws/relay`。

## 为什么 Relay Server 要留空

非空静态 Relay 字段会让客户端使用该地址，并可能绕过 HBBS 返回的动态 Relay。要验证
Starry 规则和故障切换，应保持为空。

若排障时临时填写 Relay，验证 Geo 分配前必须删除。

## WebSocket 组合

| 客户端 A | 客户端 B | 信令/数据预期 |
| --- | --- | --- |
| 关 | 关 | 原生信令，P2P 优先；需要时使用原生 HBBR。 |
| 开 | 关 | A 使用 WSS，B 保持原生；通过同一 HBBR 混合传输，且只走 Relay。 |
| 关 | 开 | 与上项相同，方向相反。 |
| 开 | 开 | WSS 信令与 WSS Relay；不走常规 P2P。 |

服务器 `websocket_signal.enabled` 只允许该路径，不会修改客户端开关；客户端开关也无法
弥补缺失的 `/ws/id`、`/ws/relay` 或无效证书。

## API 登录

基础自建远程控制不强制 API，Starry 也不提供 API。使用 API 时：

1. 独立验证 HTTPS API 状态与登录；
2. 确认 API 暴露的 HBBS 公钥和 ID Server 与客户端预期相同；
3. Relay Server 留空以便 Geo 分配；
4. 登录后重复远程控制，因为认证客户端可能在打洞或 Relay 前先在 `21116/TCP` 协商 Secure TCP。

API 登录成功不能证明 Secure TCP、HBBR 或桌面数据链路正常。

## 第一组验收客户端

使用两台已知设备并记录：

- 客户端版本和平台；
- 两端配置的 ID/API/Key，秘密必须脱敏；
- HBBS 观察到的公网出口地址；
- 两端 WebSocket 开关；
- 最终会话是 P2P 还是 Relay；
- 选中 Relay 域名和共同 Relay UUID；
- 用于对齐两端及服务器日志的时间戳。

先测试原生，再测试 API 登录，最后测试 WSS；每次只改变一个维度。

## 常见错误

- 复制 `id_ed25519` 私钥，而不是 `.pub` 公钥内容。
- 把 API access token 或商业许可证 Key 当成服务器 Key。
- 某一端遗留旧静态 Relay 地址。
- 服务器路径未完成却只在客户端打开 WebSocket。
- 认为 WSS 应保留常规 P2P；它有意只走 Relay。
- 两端位于同一 NAT 后，却没有在 `test-geo` 中把同一公网 IP 填两次。
- 对比不同连接尝试的日志，而不是同一时间戳会话。

下一步阅读
[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。
