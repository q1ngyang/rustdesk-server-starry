# 更新日志

[English](CHANGELOG.md) | **简体中文**

本文记录 Starry overlay 的变化。完整产物版本由官方 RustDesk Server 版本与 Starry
patch 版本组合，例如 `1.1.16-patch-v1.2.0`。

## patch-v1.2.0 — 2026-08-20

完整说明：[`RELEASE-NOTES-patch-v1.2.0.zh-CN.md`](RELEASE-NOTES-patch-v1.2.0.zh-CN.md)

### 新增

- schema v3 与严格 Ed25519 连接 JWT 验证；统一覆盖原生 TCP、Secure TCP、WSS 的发起端
  `PunchHoleRequest` 和直接 `RequestRelay`，并提供 `off`、`audit`、`enforce` 模式。
- JWKS 原子 last-known-good 更新、有界 token/introspection 缓存、配置远程 JWKS 或
  introspection 时强制使用排他 CA 的 mTLS，以及 key 过期、依赖故障或 subject 异常时
  fail closed。
- 不可变 Relay runtime snapshot 与返回结构化 decision trace 的无副作用分配模拟。
- 有 frame 上限、仅 loopback 的结构化本地控制协议。
- 可选 Linux `starry-control-agent`：强制 mTLS、URI SAN 白名单、短期细粒度 service
  JWT、固定 Control API v1，并默认采用安全只读 profile。
- 带精确字节 ETag、持久审计、revision history、原子写入、runtime activation ack 与不确定
  状态阻断恢复的乐观并发、幂等 plan/apply/rollback 配置事务。
- 版本化 OpenAPI 3.1、JSON/UI Schema、JWT/协议 fixtures、Control Agent
  Compose/systemd/DEB 制品及安全集成测试。

### 修复

- Relay 探测结果或就绪状态变化时推进 `health_snapshot_id`，使 Relay inventory 与同次
  allocation simulation 能唯一标识实际健康快照，而不是只标识健康配置 generation。
- 将远程 JWKS/introspection mTLS HTTP pool 的 idle lifetime 限制为 15 秒，避免重用已被
  Kessoku 较短 server idle timeout 关闭的 keep-alive 连接，并保留原有 last-known-good/
  fail-closed 行为。

### 依赖与构建完整性

- 审核后的 Cargo 依赖图已锁定；在执行任何 locked metadata、测试或 release build 前，
  workflow 会把同一锁文件复制到 patched upstream tree，并记录 upstream 与 `hbb_common`
  的精确源码提交。
- 使用 bundled SQLite 的 `tokio-rusqlite` 替换旧 SQLx/deadpool 路径，删除未固定的 Git
  `reqwest` 输入，并更新 TLS、WebSocket、JWT、protobuf 与 CLI 依赖路径。
- 固定依赖图的 RustSec 审计为 0 vulnerability、0 unsound、0 yanked；仍披露一个来自
  upstream core 的 `sodiumoxide 0.2.7` unmaintained warning，须在发布风险审核中明确接受。
- 候选 CI 将 Rust、cross、cross image、advisory 数据、扫描器、base image 和全部 Action
  固定到审核后的不可变输入；Debian 包连续构建两次并逐字节比较后才可能批准发布。

### 兼容性与安全性

- schema v1/v2 继续有效；只有 schema v3 显式开启连接认证，或部署层 `--must-login`
  floor 要求时才进入 enforce。
- 无效 reload 保留当前 last-known-good generation，不再清空 Starry runtime state。
- 认证复用既有 RustDesk protobuf 字段，未修改任何 `.proto`。UDP 连接发起继续
  unsupported，且不得触发分配。
- 官方 1.1.16 的原生被控端注册/心跳仍走 UDP；TCP/Secure TCP 覆盖的是控制端连接发起。
  需要禁用 UDP 的被控端必须使用 WSS 注册，不能把 upstream TCP `NOT_SUPPORT` 当成认证
  回归或 TCP-only 注册能力。
- HBBR 和账户/API 职责不变；Control Agent 是独立管理组件，不是账户 API 或 HBBR proxy。
- patch-v1.2.0 的可写 Control Agent 事务只在 Linux 上支持并纳入发布测试；不发布
  Windows Agent 制品。

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
