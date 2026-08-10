# 更新日志

[English](CHANGELOG.md) | **简体中文**

本文记录 Starry overlay 的变化。完整产物版本由官方 RustDesk Server 版本与 Starry
patch 版本组合，例如 `1.1.16-patch-v1.1.0`。

## patch-v1.1.0

完整说明：[`RELEASE-NOTES-patch-v1.1.0.zh-CN.md`](RELEASE-NOTES-patch-v1.1.0.zh-CN.md)

### 新增

- 与 RustDesk 客户端身份注册兼容的可选持久 `/ws/id` 信令。
- WSS↔WSS 与 WSS↔原生信令；任一端使用 WebSocket 时明确只走 Relay。
- 针对 `native`、`wss`、`mixed` 会话的传输感知 Relay 选择。
- 验证证书链和域名的 `wss://.../ws/relay` 健康探测。
- schema v2：可信代理 CIDR、精确 Origin 白名单、session/队列/限速与 Relay endpoint 完整覆盖。
- `websocket-status` 管理命令。
- `test-geo <IP_A> <IP_B> [native|wss|mixed]` 可选传输参数。
- 真实进程 WebSocket 与官方 HBBR 混合传输集成测试。

### 兼容性

- schema v1 继续有效，并保持 WebSocket Signal 关闭。
- 客户端关闭 WebSocket 时，默认行为仍是原生信令/P2P。
- 本 patch 不修改 HBBR。
- 回滚到 patch-v1.0.0 时必须恢复 schema v1 配置。

## patch-v1.0.0

### 新增

- 外部 Starry YAML 配置与严格整体验证；无效时安全回退到上游行为。
- 根据连接双方国家、城市、子区域、GeoNames ID、ASN 与 ISP 有序选择 Relay。
- `/`（OR）、`+`（AND）、括号和引号值组成的嵌套 GEO 表达式。
- 对称与方向敏感的双端规则。
- MMDB 定时下载、体积/标记/可读性校验、原子替换与最后可用版本保留。
- 原生 `21116/TCP` 上兼容 RustDesk 的 HBBS Secure TCP 协商与 Secretbox
  传输，包括认证失败关闭和合法明文首帧兼容回退。
- 本机配置重载、Relay 列表、Geo 重载和双 IP 规则测试命令。

### 兼容性

- Starry 配置为空、无法解析或验证失败时，HBBS 使用官方命令行行为。
- 没有规则命中、所需 MMDB 缺失或规则 Relay 不在线时，会继续后续规则并最终进入官方选择逻辑。
- overlay 仅修改 HBBS。

## 版本规则

- `X`：不兼容的配置/行为变更，或新的重大功能族。
- `Y`：向后兼容的功能版本。
- `Z`：当前 patch 线的紧急修复。
- 锁定的官方 RustDesk Server 版本变化时，即使 Starry patch 版本不变，完整版本前缀也会变化。

修改任一部分前请阅读
[`版本升级与回滚`](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)。
