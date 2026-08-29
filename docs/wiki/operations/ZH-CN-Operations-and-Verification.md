# 运维与完整验证

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification) | **简体中文**

容器健康、端口开放、YAML 有效或 HTTP `101` 都只能证明其中一层。生产验收必须从
配置一直验证到两台真实客户端的桌面会话。本页给出可重复的顺序和明确停止条件。

## 证据层级

| 层级 | 能证明 | 不能证明 |
| --- | --- | --- |
| 静态 | Compose、Nginx 和配置语法有效。 | 进程能启动或路由可达。 |
| 进程 | HBBS/HBBR 运行，持久化文件存在。 | RustDesk 协议注册或 Relay 流量。 |
| 网络 | DNS、TCP/UDP 和 TLS endpoint 可达。 | Peer 身份或消息交换正确。 |
| 协议 | 客户端注册并得到 Relay 分配。 | 桌面数据在负载下持续可用。 |
| 会话 | 两台真实客户端走完预期路径。 | 故障切换与恢复。 |
| 韧性 | 故障与恢复符合设计。 | 后续版本；每次重大变更后都要重测。 |

验收记录应严格区分：**通过**、**失败**、**未测试**。不得把“未测试”写成“通过”。

## 1. 记录预期状态

启动或变更服务前记录：

```sh
docker compose --env-file .env -f compose.yaml config --images
docker compose --env-file .env -f compose.yaml config > rendered-compose.review.txt
sha256sum .env compose.yaml data/starry/config.yaml > deployment-inputs.sha256
docker image inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.2 \
  --format '{{json .RepoDigests}}'
```

保留渲染后的 Compose 文件前请先审阅：它可能包含来自 `.env` 的值。密钥和验收证据
应存入受保护的运维位置，不能提交到仓库或 Issue。

同时记录官方上游版本、Starry patch 版本、架构、公开 HBBS/HBBR 域名和预期客户端
测试矩阵。

## 2. 静态校验

```sh
docker compose --env-file .env -f compose.yaml config --quiet
sudo nginx -t
```

中心/Relay 拓扑需要逐份校验：

```sh
docker compose --env-file examples/center/.env \
  -f examples/center/compose.bootstrap.yaml config --quiet
docker compose --env-file examples/center/.env \
  -f examples/center/compose.yaml config --quiet
docker compose --env-file examples/relay/.env \
  -f examples/relay/compose.yaml config --quiet
```

在 bootstrap 生成 `id_ed25519.pub`、可选 API 挂载能够解析前，不要启动完整中心文件。
Compose 成功渲染不会检查绑定挂载的公钥文件是否已经存在。

## 3. 进程和持久化检查

```sh
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --tail 200 hbbs hbbr

test -s data/id_ed25519
test -s data/id_ed25519.pub
test -f data/starry/config.yaml
test -f data/starry/config.example.yaml
```

预期结果：

- HBBS 和 HBBR 持续运行，没有反复重启；
- HBBS 健康检查变为 healthy；
- Starry 日志明确说明外部配置是否被接受；
- HBBR 与 HBBS 共用同一持久化公私钥；
- 受控重建容器后，密钥和数据库文件仍然存在。

安全备份 `id_ed25519`，只分发 `id_ed25519.pub`。生成新私钥会改变服务端身份，
使用旧公钥的客户端将无法正常连接。

## 4. 原生网络检查

从服务器局域网之外验证 TCP：

```sh
nc -vz id.example.com 21115
nc -vz id.example.com 21116
nc -vz relay.example.com 21117
```

Windows PowerShell：

```powershell
Test-NetConnection id.example.com -Port 21116
Test-NetConnection relay.example.com -Port 21117
```

UDP `21116` 需要通过注册/心跳日志或抓包验证；TCP 端口测试不能验证 UDP。检查防火墙
和云安全组的双向规则，并将测试时间与 HBBS 日志对应。

若公网入口设计为 Nginx，不要直接公开 `21118` 或 `21119`；应通过绑定或防火墙把它们
限制在反向代理路径。

## 5. TLS 和 WebSocket 路径检查

先检查 DNS 和公开证书：

```sh
openssl s_client -connect id.example.com:443 \
  -servername id.example.com -verify_return_error </dev/null

openssl s_client -connect relay.example.com:443 \
  -servername relay.example.com -verify_return_error </dev/null
```

再用 HTTP/1.1 检查精确路径：

```sh
curl --http1.1 -i --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://id.example.com/ws/id

curl --http1.1 -i --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://relay.example.com/ws/relay
```

`101 Switching Protocols` 只证明 TLS 终止和 Upgrade 路由。Curl 不会发送 RustDesk
注册或 Relay 握手，因此后续超时不是实际会话失败，而最初的 `101` 也不是完整证明。

通过已认证的 Control Agent `GET /control/v1/status` 响应查看 Starry 经证书校验的
Relay 探测。

启用或修改 endpoint 后至少等待一个配置的探测间隔。用于 `wss` 的每台 Relay 都应
健康；`mixed` 还要求同一台 Relay 同时处于原生在线状态。

## 6. Starry 配置和 Geo 检查

使用已认证的 Control Agent 方法：`POST /control/v1/runtime:reload`、
`GET /control/v1/relays`，以及带两端实际地址、transport、expected generation 和
`explain: true` 的 `POST /control/v1/allocations:simulate`。

把保留地址换成 HBBS 实际观测的两端公网地址。启用对应路径时，再用 `wss` 和
`mixed` 重复分配模拟。运行前先写下预期规则和 Relay。

## 7. 原生客户端验收

两台客户端配置相同 ID Server 和公钥；Relay Server 留空，以便 HBBS 分配；本阶段
关闭 WebSocket。

逐项执行并记录：

1. 两端都显示就绪，并出现在 HBBS 日志；
2. 网络允许时，P2P 直连可用；
3. 强制 Relay 测试通过 `21117/TCP` 到达预期 HBBR；
4. 键盘、鼠标、剪贴板及环境需要的功能持续工作一段合理时间；
5. 关闭会话后可以重新连接。

如部署第三方 API，应分别验证 API 登录，再验证登录后的原生会话。`/api/status` 或
登录成功不能证明 HBBS Secure TCP 成功；需要对应 `21116/TCP` 握手和客户端会话日志。

## 8. WebSocket 和混合验收

原生基线通过后才在客户端启用 WebSocket。逐行独立测试：

| 客户端 A | 客户端 B | Relay 必须满足 | 预期结果 |
| --- | --- | --- | --- |
| WSS | WSS | WSS 健康 | 会话使用 HBBR 并保持可用。 |
| WSS | 原生 | 原生在线 **且** WSS 健康 | 混合 Relay 会话成功。 |
| 原生 | WSS | 原生在线 **且** WSS 健康 | 反向混合方向同样成功。 |
| 原生 | 原生 | 原生在线 | 现有原生行为不变。 |

每一行都应保留：

- 两端客户端带时间戳的日志片段；
- HBBS 注册/路由日志；
- 同一 Relay 会话对应的 HBBR 日志；
- 至少一项桌面/控制可用观察结果。

只有 HTTP `101`、没有客户端 `RegisterPk` 和完整 HBBR 会话，不算通过。

## 9. 故障切换与恢复

必须在维护窗口内，只停止已获准测试的 Relay。不要通过干扰无关生产节点测试切换。

对每种适用传输方式：

1. 在第一优先 Relay 建立基线；
2. 以受控、可逆方式停止或阻断该 Relay；
3. 等待官方原生状态和/或已配置的 WSS 失败阈值；
4. 确认分配模拟改为下一台有序且符合要求的 Relay；
5. 新建真实会话并确认到达备用节点；
6. 恢复第一台 Relay；
7. 等待成功阈值；
8. 确认新会话恢复选择第一优先 Relay。

Relay 丢失时现有会话可能终止；除非自己的高可用设计承诺更多，本验收目标是新会话
被正确重新分配。

## 10. 连接认证门禁

按[连接认证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication)
执行完整矩阵。从 schema v3 `mode: audit` 开始，在每个 native/Secure/WSS
`PunchHoleRequest` 与直接 `RequestRelay` 前后采集 status。确认 would-deny 增长但不改变
预期会话结果，并确认 missing/invalid 请求既不会到达真实 target，也不会分配 Relay。

支持的真实 client 未覆盖 P2P、Relay、WSS/WSS、两个 mixed 方向、logout/revoke/disable/
password-reset、key rotation、introspection failure/recovery 与 UDP no-allocation 前，不得把
enforce 标为 ready。unit/synthetic transport test 是必要发布证据，但不能替代部署矩阵。

## 11. Control Agent 与配置恢复

以 `write_enabled: false` 接入 Agent。验证 mTLS CA/SAN 失败、service-JWT audience/azp/
scope/expiry 失败、读取 endpoint 与重复无副作用 simulation。staging 开启写入后，测试 ETag
mismatch、重复/冲突 idempotency key、plan expiry、带匹配 runtime ack 的 apply、作为新
revision 的 rollback、apply 中 HBBS outage 与 Agent restart recovery。

任何 `manual_intervention_required`、disk/runtime drift、audit intent 缺失，或 apply response
没有匹配 generation/digest 都是硬阻断；再次接受写入前按
[Control Agent 恢复 runbook](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)
处理。

## 12. 验收记录

每个版本或重大配置变更使用如下表格：

| 检查 | 预期 | 结果 | 证据/时间 |
| --- | --- | --- | --- |
| Compose 和 Nginx 语法 | 有效 | 未测试 | |
| HBBS/HBBR 稳定、密钥持久 | 是 | 未测试 | |
| 原生注册 | 两台客户端 | 未测试 | |
| 原生 P2P | 网络允许时 | 未测试 | |
| 原生 Relay | 预期 HBBR | 未测试 | |
| 登录后的 Secure TCP | 使用 API/登录时 | 未测试 | |
| WSS 到 WSS | 预期 HBBR | 未测试 | |
| WSS 到原生 | 两个方向 | 未测试 | |
| Geo 决策 | 测试矩阵每一行 | 未测试 | |
| 原生故障切换/恢复 | 按顺序 | 未测试 | |
| WSS/混合切换/恢复 | 按顺序 | 未测试 | |
| 连接认证 audit 矩阵 | 每个预期 decision 均分类 | 未测试 | |
| 连接认证 enforce canary | 无绕过，合法矩阵通过 | 未测试 | |
| JWT key/introspection 故障与恢复 | fail closed，恢复无需重启 | 未测试 | |
| 分配模拟纯度 | 不改变生产 state/counter | 未测试 | |
| Agent 只读 mTLS/RBAC | 每个 allow/deny case | 未测试 | |
| 配置 apply/rollback/outage recovery | ack 或显式安全 rollback/block | 未测试 | |
| 备份恢复演练 | 恢复密钥/配置/状态 | 未测试 | |

共享证据前应脱敏 Peer ID、Token 和公网地址。不得把私钥、API 凭据或完整访问 Token
写入日志或 Issue。
