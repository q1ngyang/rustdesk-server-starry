# Starry 连接认证约定 v1

[English](profile.md) | **简体中文**

本约定适用于主控端通过 HBBS 发起连接的场景，不要求被控端登录，也不修改 RustDesk
的 protobuf 协议结构。

## 固定校验规则

- 只接受 `EdDSA` 算法。JWT 受保护头必须包含精确的令牌类型 `typ: at+jwt`，并通过
  显式 `kid` 找到一把 `OKP`/`Ed25519` 公钥。禁止使用对称密钥、私有 JWK、算法降级，
  或遍历全部密钥尝试验签。
- 必需字段为 `iss`、`aud`、`token_use`、`scope`、`sub`、`user_id`、`auth_version`、
  `iat`、`nbf`、`exp` 和 `jti`。`user_id` 必须是非零 JSON 整数，`sub` 必须是该整数的
  规范十进制字符串。Kessoku 的 `aud` 和 `scope` 为数组；配置要求的受众与权限必须作为
  完整元素存在，不能使用子串匹配。
- 调用远程令牌状态检查前，先在本地完成签名、签发方、受众、令牌用途、有效时间、长度、
  用户身份绑定和权限校验。
- 内存中的令牌状态检查缓存以令牌的 SHA-256 摘要作为键。原始令牌和完整 JTI 不得进入
  日志、指标标签、缓存键或审计记录。请求正文严格为 `{"token":"..."}`；有效响应必须
  包含与本地验证结果完全一致的 `sub`，缺失或不匹配都必须拒绝。
- `mode: enforce` 下，缺少有效初始密钥、密钥集过期超过配置上限，或必需的远程令牌
  状态检查不可用时，都必须拒绝连接。部署参数 `--must-login` / `MUST_LOGIN=Y`
  规定了不可降低的强制认证要求，配置和远程管理都不能将其关闭。
- `PunchHoleRequest` 和直接 `RequestRelay` 共用同一认证决策入口；认证在帧和
  protobuf 边界检查后进行，并先于目标查询、打洞请求记录、中继选择和请求转发。
- 原生 TCP、Secure TCP 和 `/ws/id` 共用同一校验器。UDP `PunchHoleRequest`
  仍不受支持，不能进入认证、目标查询或中继分配流程。

## 兼容客户端的拒绝响应

不增加 protobuf 枚举值。拒绝 `PunchHoleRequest` 时，使用既有 `PunchHoleResponse`，
设置 `failure=OFFLINE`，并在 `other_failure` 中返回固定文本
`connection authorization failed`。拒绝直接 `RequestRelay` 时，在
`RelayResponse.refuse_reason` 中返回相同文本。

内部原因码只保留在服务端审计与指标中；响应不得透露目标是否存在、用户是否被禁用或
令牌是否被撤销。

## 测试样本的固定时钟

样本验证时刻为 `2030-01-01T00:00:00Z`（`1893456000`）。有效令牌的时间窗口覆盖该
时刻；过期令牌已超出有效期；错误受众令牌虽签名有效，但必须在受众校验时失败。

样本沿用 Kessoku 的协议字段形式：签发方 `https://api.example.test`，受众
`kessoku-api` 和 `rustdesk-connect`，令牌用途 `access`，权限 `connect:initiate`，
数值用户 ID `42`，密钥 ID `kessoku-fixture-2030-01`。样本密钥仅供测试，绝不能作为
部署环境的受信任密钥。

测试样本仍位于 [`contracts/auth/v1/fixtures`](../../../../contracts/auth/v1/fixtures)。
另见[客户端兼容性参考](client-compatibility.zh-CN.md)。
