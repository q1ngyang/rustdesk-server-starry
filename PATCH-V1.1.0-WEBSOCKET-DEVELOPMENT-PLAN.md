# rustdesk-server-starry patch-v1.1.0 WebSocket 开发方案

> 文档状态：核心代码与核心自动化验收已完成；发布流水线验收以对应 GitHub Actions 运行记录为准，完整压力与真实环境验收待执行
>
> 制定日期：2026-08-10
>
> 目标版本：patch-v1.1.0
>
> 首个目标发布：1.1.16-patch-v1.1.0
>
> 开发基线：rustdesk-server-starry 5d8cb4a，官方 rustdesk-server 1.1.16
>
> 方案边界：本文件同时记录开发基线和当前实施状态；GitHub Release 只代表发布流水线通过，不代表生产环境已部署。

## 0. 实施状态（2026-08-10）

已在当前工作区完成：

- schema v2、持久 `/ws/id` 注册、单写者有界队列、会话代际替换、跨传输信令和 Relay-only 决策；
- 按 native、wss、mixed 过滤的 Geo Relay 选择，以及验证证书和主机名的 `/ws/relay` 健康检查；
- 可信代理、Origin、连接/会话/速率限制、状态命令和配置热重载；
- 官方 HBBR 原生/WSS 混合双向转发测试，并覆盖 WSS 先登记与原生先登记两种顺序；
- 对官方 `1.1.16` 干净源码重复应用 overlay，第二次无文件变化且 `git diff --check` 通过；
- Linux/Rust 1.97 环境中的库与二进制编译检查，以及 30 个库单元测试；
- 真实 HBBS 进程集成测试：使用运行时生成的测试 CA 与域名匹配证书，完成 `/ws/relay` TLS Upgrade、RegisterPeer/RegisterPk、keep_alive、空二进制心跳、WSS↔WSS、WSS→原生、原生→WSS 和 Relay-only 选择；该测试在 `RUST_LOG=warn` 下运行，锁定启动逻辑不能依赖 info 日志宏求值。

官方 HBBR 将非 WebSocket loopback 连接保留为本机管理命令通道；混合 Relay 集成测试因此使用非 loopback 本机地址连接原生端。测试还会先让第一端登记，再发送第二端请求，避免用零间隔合成流量触发官方 `PEERS` 登记竞态。两种真实连接顺序都仍被测试。

以下真实环境与容量 Definition of Done 项目尚未完成，即使 GitHub Release 成功也不能视为生产上线：

- 完整 TLS 反向代理下 `/ws/id` 与 `/ws/relay` 的端到端测试、Relay 故障后的 Geo 自动切换，以及 1,000 个空闲 WSS session/30 分钟与重连风暴压力测试；
- 两台真实 RustDesk 客户端的 Ready、桌面控制、API 登录/未登录与模式切换；
- 七个生产 Relay 的证书、反向代理、WSS/native 混合通道与共同 Relay UUID 验收；

双架构、Windows、DEB、容器、发布 Artifact、SBOM 与 GHCR manifest 属于每次正式发布流水线的强制门禁；结果以该版本 GitHub Actions 运行记录为准。生产回滚演练仍属于真实环境验收。

## 1. 结论与已确定决策

继续采用“官方 rustdesk-server + Starry 小型 overlay”的结构，在 HBBS 中补齐 RustDesk 客户端开启 WebSocket 后所需的持久化 WSS 信令能力。

patch-v1.1.0 的产品定位不是把全部客户端迁移到 WebSocket，而是提供受限企业网络中的按需兼容通道：

| 使用环境 | 客户端设置 | 信令路径 | 数据路径 |
|---|---|---|---|
| 日常网络 | WebSocket 关闭 | 原生 UDP/TCP 21116 | 优先 P2P，失败后原生 Relay 21117 |
| 企业受限网络 | WebSocket 开启 | HTTPS/WSS 443 到 /ws/id，再到 HBBS 21118 | WSS 443 到 /ws/relay，再到 HBBR 21119 |
| 一端企业、一端日常 | 一端 WSS、一端原生 | HBBS 统一跨传输路由 | 同一个官方 HBBR 完成 WSS 与原生混合 Relay |

以下决策已经固定，不在开发过程中重新发散：

1. 服务端在灰度通过后同时长期提供原生与 WSS 两条路径。
2. 客户端 WebSocket 继续默认关闭，由用户在受限网络中手动开启。
3. 只要任一端使用 WSS，本次 ID 会话就是 Relay-only，不尝试保留普通 P2P。
4. WSS 与原生客户端的双向混合会话是发布阻断项，不是可选兼容项。
5. 第一阶段只 fork HBBS；官方 HBBR 继续使用，不复制或维护 HBBR fork。
6. WSS/TLS 不进入 Secure TCP KeyExchange；现有 21116 Secure TCP 实现保持独立。
7. 所有可能被 Geo 选中的生产 Relay 都必须同时提供原生 21117 和 WSS 443 /ws/relay。
8. 不复制 RustDesk Server Pro 闭源实现；只依据公开客户端协议、官方 AGPL 源码及兼容实现进行开发。

## 2. 问题定义

现有官方 OSS HBBS 已监听 WebSocket ID 端口并能够完成 HTTP Upgrade，但 WSS 客户端的持久注册首帧 RegisterPk 会进入现有 handle_tcp 分支并得到 NOT_SUPPORT，随后连接关闭。客户端因此停留在“正在接入 RustDesk 网络”。

这说明缺失的不是以下部署能力：

- DNS；
- TLS 证书；
- HTTPS 443；
- Nginx Upgrade；
- HBBR 21119 WebSocket listener。

真正缺失的是 HBBS 的以下业务能力：

- WSS 持久注册；
- RegisterPk 身份校验和成功响应；
- 客户端可识别的空二进制心跳；
- Peer ID 到 WSS 会话的绑定；
- WSS、UDP 和原生 TCP 之间的统一信令转发；
- 同一客户端切换传输方式后的原子替换和安全清理；
- WSS 感知的 Geo Relay 过滤与健康检查。

只把 NOT_SUPPORT 改成 OK 会让客户端短暂显示 Ready，但仍无法接收来自其他 Peer 的 PunchHole、RequestRelay 等消息，也不能证明远程控制可用，因此禁止采用这种伪修复。

## 3. 目标与非目标

### 3.1 必须实现

1. RustDesk 1.4.0 及以上客户端开启 WebSocket 后，可以通过 /ws/id 持久注册。
2. API 登录和未登录客户端都能完成 WSS 注册；WSS 不触发 Secure TCP 双重握手。
3. WSS 到 WSS、WSS 到原生、原生到 WSS 三类信令和 Relay 会话均可工作。
4. 客户端关闭 WebSocket 后，原生 P2P、原生 Relay 和现有 Secure TCP 行为不变。
5. 同一个设备从 WSS 切换回原生或从原生切换到 WSS 时，不遗留幽灵路由。
6. Geo 只能给 WSS 会话选择真正可用的 /ws/relay 节点。
7. 每个连接都有上限、超时、背压和可追踪但不泄密的错误分类。
8. overlay 在干净上游源码上可重复应用两次；锚点变化时明确失败。
9. Linux amd64、Linux arm64、Windows amd64、DEB 和双架构容器继续构建。

### 3.2 明确不实现

- 自动判断客户端所处网络并远程切换客户端 WebSocket 设置；
- 在 WSS 会话中恢复普通 UDP/TCP P2P；
- 复制 RustDesk Server Pro 的账户、ACL、审计或管理控制台；
- 修改 RustDesk 客户端；
- 第一阶段 fork HBBR；
- 用不校验证书的 TLS 或明文 ws 代替生产 WSS；
- 用 GeoDNS、轮询 DNS 或不同 HBBR 实例配对同一个 Relay UUID；
- 把 Web Remote 控制台的 /ws/relay 与原生 RustDesk HBBR 21119 混为一条路由。

## 4. 架构

    普通客户端，WebSocket 关闭
      ├─ UDP/TCP 21116 ───────────────────────────────┐
      └─ 原生 Relay 21117 ────────────────────────┐   │
                                                   │   ▼
    企业客户端，WebSocket 开启                     │  Geo HBBS
      ├─ WSS 443 /ws/id → 21118 ──────────────────┤   ├─ 原生 PeerMap
      └─ WSS 443 /ws/relay → 21119 ────────┐       │   ├─ WSS Session Registry
                                            ▼       │   ├─ Cross-Transport Router
                                      官方 HBBR ◀──┘   └─ Geo/Relay Eligibility
                                      同一进程同时处理
                                      原生与 WSS Relay

新增代码应分为三层：

| 层次 | 责任 | 禁止耦合 |
|---|---|---|
| websocket_signal transport | Upgrade 后的读写、帧限制、心跳、会话生命周期 | 不包含 Geo 规则 |
| peer routing | WSS 与现有原生 PeerMap 之间的投递和替换 | 不包含 TLS 探测 |
| relay transport health | WSS endpoint 探测及 Relay 能力过滤 | 不处理 RegisterPk |

推荐新增目录：

    overlay/src/
    ├─ websocket_signal.rs
    └─ websocket_signal/
       ├─ session.rs
       ├─ routing.rs
       └─ relay_health.rs

继续保留：

    overlay/src/secure_tcp.rs
    overlay/src/geo_relay.rs
    overlay/src/geo_relay/rules.rs
    overlay/src/starry_config.rs

secure_tcp.rs 不应因 patch-v1.1.0 修改协议状态机；如果为统一 Sink 类型必须调整，只允许做机械适配并执行全部 v1.0.0 回归。

## 5. WebSocket 连接角色与状态机

不能假设所有 /ws/id 连接都以 RegisterPk 开始。RustDesk 既有持久注册连接，也可能建立短连接发送 PunchHoleRequest、RelayResponse、TestNatRequest 等现有 TCP 信令。

每个 Upgrade 成功的连接按第一条有效二进制消息分流：

    Upgraded
       │
       ├─ 注册超时、文本帧、超大帧或畸形 protobuf
       │      └─ Close
       │
       ├─ RegisterPk 或兼容的 RegisterPeer
       │      ├─ 身份校验失败 ──> 明确响应后 Close
       │      └─ 身份校验成功 ──> PersistentRegistered
       │                              ├─ 心跳
       │                              ├─ 接收和路由信令
       │                              └─ 超时、替换或关闭时安全清理
       │
       └─ 允许的一次性信令
              └─ EphemeralDispatch
                     ├─ 复用上游 handle_tcp 业务逻辑
                     └─ 按上游返回值继续读取或关闭

### 5.1 PersistentRegistered 要求

- 返回客户端支持的 RegisterPkResponse，包含正确 result 和 keep_alive。
- 心跳使用客户端能够观察和回送的空二进制帧，不只依赖 WebSocket Ping/Pong。
- keepalive_interval_ms 必须严格小于 idle_timeout_ms。
- 收到有效空二进制回送时更新 last_seen。
- Close、读取错误、写入错误、空闲超时、配置停用和会话替换都进入统一清理路径。
- 清理时必须携带 generation，旧连接不能删除后来建立的新连接。

### 5.2 EphemeralDispatch 要求

- 继续复用现有 handle_tcp 业务语义，避免重新实现两套 PunchHole/Relay 协议。
- 短连接不得被写入持久 Session Registry。
- 一次性 Sink 和持久 Sink 使用相同的异步写入抽象，但生命周期不同。
- 上游原有返回 true 或 false 的读取语义保持不变。

## 6. 写入所有权、并发和背压

WebSocket SplitSink 只能由单一 writer task 持有。禁止把 Arc<Mutex<WsSink>> 分发给多个任务并在持锁状态下执行网络 await。

推荐结构：

    struct WsWriteTransport {
        tx: mpsc::Sender<OutboundFrame>,
        connection_id: u64,
    }

    struct WsSessionHandle {
        peer_id: String,
        generation: u64,
        tx: mpsc::Sender<OutboundFrame>,
        effective_ip: IpAddr,
        connected_at: Instant,
        last_seen_millis: AtomicU64,
        cancellation: CancellationToken,
    }

约束：

- outbound channel 必须有界。
- last_seen_millis 使用同一进程内的单调时钟基准，不使用可能回拨的墙上时间。
- 队列满时记录 slow_consumer 分类并关闭该连接，不能无限缓存。
- 业务锁、Session Registry 锁和 PeerMap 锁都不得跨网络 await。
- 每个 Peer ID 同时只允许一个当前持久 WSS generation。
- 新的、已完成身份校验的注册可以替换旧 generation。
- 未通过校验的 RegisterPk 不得驱逐现有 WSS 或原生路由。
- writer task 退出必须通知 reader 和 registry 清理；reader 退出也必须取消 writer。

现有 Sink 枚举的 WSS 分支应从原始 WsSink 改为 WsWriteTransport。原生分支继续使用 secure_tcp::TcpWriteTransport。

## 7. 身份注册与 Peer 路由

### 7.1 复用 RegisterPk 安全语义

必须把现有 UDP/TCP RegisterPk 校验抽取为共享业务函数，至少统一：

- ID 格式；
- UUID；
- 公钥长度与变化规则；
- 数据库读写；
- UUID mismatch 等响应；
- 频率限制；
- 日志脱敏。

WSS 不得维护一套更宽松的身份规则。

### 7.2 不使用伪 SocketAddr

WSS 连接的 Nginx 源端口不能代表客户端 NAT 端口。禁止为了塞入现有 PeerMap 而构造 effective_ip:0 或其他伪 SocketAddr，再让它进入打洞逻辑。

推荐增加逻辑路由门面：

    enum SignalRoute {
        Native,
        WebSocket {
            generation: u64,
            effective_ip: IpAddr,
            tx: WsWriteTransport,
        },
    }

物理实现可以继续让原生 PeerMap 归上游管理，并由 WSS sidecar registry 只保存 WSS 路由。统一投递函数按以下顺序工作：

1. 查询当前 Peer 是否有有效 WSS route。
2. 有则投递到有界 writer channel。
3. 没有则调用现有 UDP/TCP 投递路径。
4. native 注册成功时，原子驱逐该 Peer 的旧 WSS route。
5. WSS 注册成功时，把当前逻辑 route 设为 WSS，但不伪造可用于 P2P 的地址。

### 7.3 传输切换

同一客户端在不同网络中可能反复切换 WebSocket：

- WSS → 原生：新的有效 native 注册胜出，旧 WSS generation 被取消。
- 原生 → WSS：新的有效 WSS 注册胜出，后续信令投递 WSS。
- 旧 WSS reader 延迟退出：只能清理自己的 generation。
- WSS 与 native 心跳短暂交叠：以最后一次完整身份校验和绑定为准，而不是最后一个任意数据包。

必须为以上四种竞争条件编写确定性测试。

## 8. 跨传输信令

至少覆盖客户端持久 TCP/WSS 路径实际处理的消息：

| 消息 | 方向 | 处理要求 |
|---|---|---|
| RegisterPk / RegisterPeer | Client → HBBS | 校验、绑定 route、返回成功或明确错误 |
| PunchHoleRequest | Controller → HBBS | 查找目标 route；任一端 WSS 时强制 Relay |
| PunchHole | HBBS → Target | 可投递到 WSS 或现有原生通道 |
| RequestRelay | HBBS → Target | 两端收到同一个 Relay 地址和 UUID |
| RelayResponse | Client ↔ HBBS/Peer | 跨 WSS 与原生路由 |
| PunchHoleSent | Client → Peer | 跨传输投递 |
| FetchLocalAddr / LocalAddr | Client ↔ Peer | 保持上游语义 |
| TestNatRequest | Client → HBBS | 直接响应，不建立持久 Peer route |
| OnlineRequest | Client → HBBS | 保持现有在线查询语义 |
| 空二进制帧 | 双向 | 仅作为持久连接心跳 |

当任一端使用 WSS 时：

1. 不发送会诱导任一端进行普通 P2P 的组合消息。
2. HBBS 选择一个同时满足双方传输要求的具体 Relay。
3. 两端收到完全相同的 Relay 地址和 Relay UUID。
4. WSS 端通过 /ws/relay 进入该节点，原生端通过该节点 21117 进入同一个 HBBR 进程。

## 9. 官方 HBBR 边界

官方 rustdesk-server 1.1.16 的 HBBR 用统一 StreamTrait 配对。只有两端都是原生传输时才切为 raw；存在 WSS 端时继续由各自 transport 完成帧适配。因此第一阶段保持官方 HBBR。

这项源码判断不能替代运行验收。必须新增自动化集成测试：

1. 启动未修改的官方 HBBR。
2. 客户端 A 通过 WebSocket 21119 发送 RequestRelay UUID。
3. 客户端 B 通过原生 21117 发送相同 UUID。
4. A → B 和 B → A 分别发送非空二进制载荷并校验完全一致。
5. 调换先后顺序重复测试。
6. 覆盖断开、配对超时和 UUID 不匹配。

如果固定基线的官方 HBBR 无法通过混合测试，应立即停止 patch-v1.1.0 开发并报告证据。不得在同一范围内静默增加 HBBR fork，也不得把“要求两端都开启 WebSocket”作为发布替代方案。

## 10. Geo 与 Relay 传输能力

### 10.1 传输要求

新增：

    enum RelayRequirement {
        NativeOnly,
        WebSocketOnly,
        Mixed,
    }

选择规则：

| 双方信令模式 | Requirement | Relay 条件 |
|---|---|---|
| 原生 + 原生 | NativeOnly | 继续使用当前在线 Relay 列表 |
| WSS + WSS | WebSocketOnly | /ws/relay 的 TLS、Upgrade 和 HBBR listener 健康 |
| WSS + 原生 | Mixed | 同一节点的 WSS endpoint 与原生 21117 都健康 |

geo_relay::select_relay 必须接收 Requirement，继续保持规则顺序：

1. 按 Geo 表达式找到匹配规则。
2. 按规则 relays 顺序逐个检查传输资格。
3. 第一个满足 Requirement 的节点胜出。
4. 当前规则没有合格节点时继续下一规则。
5. WSS/Mixed 最终没有合格节点时返回明确的 no_eligible_websocket_relay，不得回退到已知只支持原生的节点。

NativeOnly 的现有行为和测试必须保持不变。

### 10.2 WSS Relay 健康检查

现有原生在线状态不足以代表 WSS。新增 relay_health.rs，分别保存：

- native health：继续来源于当前上游可分配 Relay 集合；
- websocket health：Starry 主动探测；
- last_success；
- last_failure；
- consecutive_successes；
- consecutive_failures；
- 当前状态 Healthy、Unhealthy 或 Unknown。

WSS 探测必须验证：

1. DNS；
2. TCP 443；
3. TLS 握手；
4. 证书链和域名匹配；
5. HTTP WebSocket Upgrade 到精确 /ws/relay；
6. 收到 101 后立即发送规范 Close 并释放连接。

探测不发送真实 RequestRelay UUID，因此只证明 endpoint 可进入 HBBR WebSocket listener，不等同于完整远控验收。

禁止：

- 跳过 TLS 证书验证；
- 用 ping 或 Mihomo 198.18.x.x Fake-IP 代替 Relay 探测；
- 只测 21117 后推断 WSS 正常；
- 用一个普通 HTTPS 200 页面代替 /ws/relay Upgrade。

## 11. 配置 schema v2

patch-v1.1.0 引入 schema version 2。二进制必须同时读取：

- version: 1：按 patch-v1.0.0 语义加载，WebSocket Signal 默认关闭；
- version: 2：允许 websocket_signal 配置；
- 空文件或无效文件：继续完整回退上游行为。

建议配置：

    version: 2

    relay_servers:
      - jp-relay-1.example.com:21117
      - us-relay-1.example.com:21117

    websocket_signal:
      enabled: true
      registration_timeout_ms: 10000
      keepalive_interval_ms: 12000
      idle_timeout_ms: 45000
      max_frame_bytes: 65536
      outbound_queue_capacity: 64
      max_sessions: 10000
      max_sessions_per_effective_ip: 512
      registration_rate_per_minute: 300
      trusted_proxies:
        - 127.0.0.1/32
        - ::1/128
      allowed_origins: []
      relay_health:
        interval_seconds: 60
        timeout_ms: 5000
        success_threshold: 1
        failure_threshold: 2
        endpoints:
          - relay: jp-relay-1.example.com:21117
            url: wss://jp-relay-1.example.com/ws/relay
          - relay: us-relay-1.example.com:21117
            url: wss://us-relay-1.example.com/ws/relay

校验要求：

- enabled 为 true 时，每个 relay_servers 条目必须恰好存在一个 endpoint。
- endpoint.relay 必须与 relay_servers 规范化后完全匹配。
- 生产 URL 必须使用 wss。
- URL 必须使用明确主机名、精确 /ws/relay 路径且不得带凭据。
- 不允许重复 relay 或重复 URL。
- keepalive_interval_ms 小于 idle_timeout_ms。
- registration_timeout_ms、idle_timeout_ms、frame、queue、session 总量、单 IP session 和注册速率都有合理上下界。
- trusted_proxies 必须是可解析 CIDR。
- allowed_origins 为空时，接受无 Origin 的原生 RustDesk 客户端，但拒绝带任意浏览器 Origin 的连接。

现有 v1 配置不得被 v1.1.0 自动重写。用户显式升级到 v2 后，部署流程必须同时保存：

- config.v1.backup.yaml；
- config.v2.yaml；
- 当前镜像 digest。

回滚到 patch-v1.0.0 时必须同时恢复 v1 配置；旧二进制遇到 v2 配置会按既有规则判为无效并回退上游，不能把这种回退误认为 Geo 和 Secure TCP 仍然启用。

## 12. 可信代理、来源 IP 与 Origin

Nginx 传入的 X-Real-IP 和 X-Forwarded-For 只有在 TCP peer 属于 websocket_signal.trusted_proxies 时才能使用。

处理规则：

1. TCP peer 不可信：忽略全部转发头，effective_ip 使用 TCP peer IP。
2. TCP peer 可信：按明确规则解析 X-Forwarded-For，并拒绝畸形或超长链。
3. 不记录完整转发头。
4. 生产 21118 仍应通过主机防火墙只允许反向代理访问。
5. 无 Origin 视为原生 RustDesk 客户端，可接受。
6. 存在 Origin 时必须精确匹配 allowed_origins；不做后缀模糊匹配。

Geo 查询、频率限制和审计使用 effective_ip；任何 P2P socket 逻辑不得使用代理端口或伪造端口。

## 13. 配置重载和管理命令

reload-starry-config 应扩展为：

- v1 → v2：启动 WSS Relay 健康任务；新连接启用持久注册。
- v2 配置参数变化：新连接使用新限制；健康检查任务以 generation 方式重启。
- enabled true → false：停止接收新持久注册，并在可控的 drain 时间内关闭现有 WSS Session。
- enabled false → true：先完成 endpoint 配置校验和至少一轮健康探测，再标记 Ready。

新增仅限 loopback 管理端口使用的命令：

    websocket-status
    ws

输出：

- 配置是否启用；
- 当前持久 WSS session 数；
- 正在 drain 的 session 数；
- 各 Relay 的 native/WSS 健康状态及最近更新时间；
- 注册、替换、超时和 slow-consumer 计数。

输出不得包含完整 Peer ID、公钥、Token 或原始 IP 列表。

扩展现有测试命令：

    test-geo <IP_A> <IP_B> [native|wss|mixed]

省略第三个参数时保持 patch-v1.0.0 的 native 行为。

## 14. Overlay 改造清单

### 14.1 scripts/apply_overlay.py

- copy_overlay 增加 websocket_signal.rs 和 websocket_signal 目录。
- lib.rs 注入 mod websocket_signal。
- 将 WsSink 别名和 Sink::WsStream 的 raw sink 替换为 WsWriteTransport。
- 在 WSS handle_connection 分支注入第一帧分流、writer task 和 session lifecycle。
- 在 RegisterPk、原生注册成功及 Peer 投递位置设置唯一锚点。
- get_relay_server 扩展 RelayRequirement，NativeOnly 保留原逻辑。
- reload-starry-config 调用 websocket_signal::reconfigure。
- 管理命令增加 websocket-status，并给 test-geo 增加可选传输参数。
- 脚本重复应用仍必须幂等。
- 每个新锚点匹配数量不是 1 时立即失败。

### 14.2 overlay/src/starry_config.rs

- 引入 v1/v2 兼容反序列化。
- 新增 WebSocketSignalConfig、RelayHealthConfig 和 RelayEndpointConfig。
- 严格 deny_unknown_fields。
- 增加完整的范围、唯一性、覆盖率、URL、CIDR 和时序校验。
- 配置解析错误不得部分启用 WSS。

### 14.3 overlay/src/websocket_signal.rs

- 对外提供 WSS connection 驱动、注册、投递、替换、状态查询和重配置入口。
- 不直接读取 Geo/MMDB。
- 不处理 Secure TCP。

### 14.4 overlay/src/websocket_signal/session.rs

- writer task；
- reader state machine；
- heartbeat；
- timeout；
- bounded queue；
- generation-safe cleanup；
- close reason 分类。

### 14.5 overlay/src/websocket_signal/routing.rs

- WSS sidecar registry；
- try_send_to_websocket；
- native registration eviction hook；
- WSS route snapshot；
- 跨传输 delivery result。

### 14.6 overlay/src/websocket_signal/relay_health.rs

- WSS endpoint probe；
- 状态阈值；
- 配置 generation；
- RelayRequirement 过滤；
- 供 websocket-status 查询的脱敏快照。

### 14.7 overlay/src/geo_relay.rs

- select_relay 增加 RelayRequirement；
- NativeOnly 结果不得变化；
- WSS/Mixed 使用 transport eligibility；
- 日志注明规则、Relay 和 requirement，不记录完整双方 IP。

### 14.8 不应修改

- Geo 表达式 grammar；
- Geo 规则优先级；
- Secure TCP 密码学、nonce 和降级规则；
- 官方 HBBR 业务代码；
- API 登录响应；
- 数据库 schema。

## 15. 单元测试

### 15.1 配置

- version 1 继续加载，WSS disabled。
- version 2 示例完整通过。
- v1 出现 websocket_signal 字段时明确报错。
- v2 缺少 endpoint 覆盖、重复 endpoint、非 wss URL、错误路径、错误 CIDR 时拒绝整个配置。
- keepalive 大于等于 idle timeout 时拒绝。
- 空或无效配置完整回退上游。

### 15.2 状态机

- RegisterPk 成功后进入 PersistentRegistered。
- RegisterPk 各错误结果与原生路径一致。
- 一次性消息不写入 Session Registry。
- 文本、畸形 protobuf、超大帧和注册超时关闭。
- 空二进制心跳更新 last_seen。
- idle timeout 清理 registry。
- writer 错误取消 reader；reader 错误取消 writer。
- queue 满触发 slow_consumer 并释放内存。

### 15.3 并发与切换

- 新 WSS generation 替换旧 generation。
- 旧 generation 延迟退出不删除新 generation。
- 有效 native 注册驱逐旧 WSS。
- 无效 native 或 WSS 注册不能驱逐当前 route。
- WSS → native → WSS 快速切换结果确定。
- 发送期间发生 route 替换不会死锁或写入错误连接。

### 15.4 Geo 和健康

- NativeOnly 与 patch-v1.0.0 的现有测试结果完全一致。
- WebSocketOnly 跳过 native-only 节点。
- Mixed 要求同一节点两类入口都健康。
- 首选 WSS Relay 失败后选择规则中的下一节点。
- 所有规则都无合格节点时返回明确错误，不进入原生 round-robin。
- Unknown、成功阈值、失败阈值和 generation 重启转换正确。
- TLS、证书名、非 101、错误 path 和超时分别分类。

## 16. 自动化集成测试

在 CI 中新增独立测试任务，至少包括：

1. 干净官方 1.1.16 源码应用 overlay 两次。
2. 启动 HBBS，使用 tokio-tungstenite 发送合法 RegisterPk fixture。
3. 校验成功响应、keep_alive、空二进制心跳和回送。
4. 模拟两个 WSS Peer，验证 PunchHole/RequestRelay 跨 session 投递。
5. 模拟 WSS Peer 与原生 Peer，验证两个方向的信令。
6. 启动未修改的官方 HBBR，完成 WSS ↔ native 双向 payload 测试。
7. 使用测试 CA 和匹配域名启动 TLS 反向代理，验证 /ws/id 和 /ws/relay。
8. 关闭某个 /ws/relay，验证 Geo 跳过该节点。
9. 启动 1,000 个空闲 WSS 注册连接并保持至少 30 分钟，确认内存趋于稳定。
10. 模拟慢读客户端，确认有界队列生效且其他 session 不受影响。

自动化 TLS 测试必须把测试 CA 显式加入测试信任库；禁止使用 insecure 或跳过证书校验参数。

## 17. 真实客户端验收矩阵

“加入网络成功”、HTTP 101 或 Compose 静态校验都不是最终验收。

| 编号 | 控制端 | 被控端 | API | 网络 | 预期 |
|---|---|---|---|---|---|
| C01 | 原生 | 原生 | 未登录 | 普通 | P2P 成功 |
| C02 | 原生 | 原生 | 已登录 | 普通 | Secure TCP + P2P 成功 |
| C03 | 原生 | 原生 | 已登录 | 强制 Relay | Geo + 21117 成功 |
| C04 | WSS | WSS | 未登录 | 仅 443 | WSS Relay 成功 |
| C05 | WSS | WSS | 已登录 | 仅 443 | 不进入 Secure TCP，WSS Relay 成功 |
| C06 | WSS | 原生 | 混合 | 企业到普通 | 混合 Relay 成功 |
| C07 | 原生 | WSS | 混合 | 普通到企业 | 混合 Relay 成功 |
| C08 | 同一客户端 WSS → 原生 | 任意 | 已登录 | 网络切换 | 新 route 胜出，无幽灵 session |
| C09 | 同一客户端原生 → WSS | 任意 | 已登录 | 网络切换 | 新 route 胜出 |
| C10 | WSS | 原生 | 任意 | 首选 WSS 443 故障 | Geo 选择下一个合格 Relay |
| C11 | WSS | WSS | 任意 | 只有 /ws/id 正常 | 明确失败，不误报可远控 |
| C12 | 原生 | 原生 | 任意 | WSS 服务停用 | 原生功能完全不受影响 |

客户端版本：

- 最低：1.4.0；
- 当前基线：1.4.9；
- 发布时最新正式版；
- Windows 为强制平台；
- Linux 至少完成一组交叉平台控制；
- macOS、Android 和 iOS 至少做注册与一组控制 smoke test；发现协议差异时单独记录，不静默降低桌面验收标准。

每个生产 Relay 必须分别执行 C04、C06 或 C07 中至少一个真实会话。日志应按同一个 Relay UUID 对齐 HBBS、两端客户端和 HBBR，不能使用登录成功或端口连通代替数据通道证据。

## 18. 性能与容量

patch-v1.1.0 不给原生客户端引入额外网络跳转。NativeOnly 路径只允许增加轻量 route 判定，不得改变 P2P 优先级。

需要记录：

- 1,000 个空闲 WSS session 的 HBBS CPU 和 RSS；
- 单 session 近似内存；
- 心跳发送速率；
- reconnect storm 下每秒注册量；
- writer queue 满次数；
- WSS session 数、混合会话数和 Relay 选择失败数；
- 每个 HBBR 的 WSS/原生带宽。

最低稳定性门槛：

- 1,000 个空闲 session 运行 30 分钟后 RSS 不持续线性增长；
- 100 次/秒重连持续 10 分钟无死锁、panic 或 route 泄漏；
- 一个慢客户端不能阻塞其他 session；
- NativeOnly 的端到端建立时间无明显回退；
- WSS 带宽容量按 Relay-only 规划，不能按普通 P2P 流量估算。

## 19. 日志与安全

允许记录：

- connection trace ID；
- transport：native、secure_tcp、websocket；
- 状态变化和耗时；
- RegisterPk 结果枚举；
- session generation；
- RelayRequirement；
- Geo 规则名称和选中 Relay；
- 关闭原因；
- 聚合计数。

禁止记录：

- API Token；
- 服务端私钥或客户端公钥原文；
- 完整 Peer ID、UUID 或密码；
- WebSocket 原始 payload；
- 完整 X-Forwarded-For；
- 未脱敏的长期 IP 清单。

必须检查：

- 同 ID 抢占不能绕过 RegisterPk 校验；
- 代理头不能由公网直连伪造；
- Origin 规则不会拒绝无 Origin 的原生客户端；
- frame、queue、注册速率和 session 总数都有上限；
- 配置 reload 不产生旧健康任务覆盖新状态；
- panic、任务取消和连接关闭不会留下 registry entry。

## 20. CI 与发布流程

### 20.1 CI 变更

- build.yml 增加 pull_request 到 main 的验证，但 PR 永远不发布。
- rustfmt 检查加入全部 websocket_signal 文件。
- cargo test --locked --lib -j 1 继续执行全部现有和新增测试。
- cargo check --locked --bins -j 1 保持。
- 新增 HBBR mixed transport 集成任务。
- 新增本地 TLS/WSS 集成任务。
- Compose config、Linux amd64/arm64、Windows amd64、DEB 和容器 smoke test 保持。
- release image description 和 release notes 增加 WSS Signal、混合 Relay 与配置 schema v2。

### 20.2 版本规则

- 开发分支目标版本写作 patch-v1.1.0。
- PATCH_VERSION 文件在发布提交中从 1.0.0 改为 1.1.0。
- 首个正式 tag 为 1.1.16-patch-v1.1.0。
- DEB 版本沿用当前工作流生成规则。
- 后续上游自动跟随保持上游版本加 patch-v1.1.0。

### 20.3 首版发布保护

当前定时工作流可在 STARRY_RELEASE_ENABLED 为 true 时自动发布。为防止合并后未经真实客户端灰度就自动推送：

1. 开发只在 feature 分支进行。
2. PR CI 只构建和上传短期 Artifact，不更新 latest。
3. 用未发布 Artifact 或候选镜像完成隔离环境和一个 Relay 的灰度。
4. 合并和正式发布窗口前暂停定时自动发布，或增加一次性 v1.1.0 发布批准变量。
5. 全部发布阻断项通过后，手动 workflow_dispatch 指定 1.1.16 并 publish。
6. 校验 GitHub Release、SHA256SUMS、GHCR 两架构 manifest、SBOM 和容器 smoke。
7. 正式发布成功并观察后再恢复定时自动跟随。

不得让 Watchtower 在候选阶段自动把全部生产 HBBS 更新到 patch-v1.1.0。

## 21. 开发里程碑与 PR 拆分

| 里程碑 | 内容 | 预计 |
|---|---|---|
| M0 | 固定源码、协议 fixture、真实失败复现及官方 HBBR 混合 Relay 先行证明 | 1–2 天 |
| M1 | schema v2、配置校验、v1 兼容 | 1–2 天 |
| M2 | WsWriteTransport、状态机、心跳、registry | 2–3 天 |
| M3 | RegisterPk 复用、跨传输信令、切换清理 | 3–4 天 |
| M4 | RelayRequirement、WSS 健康检查、Geo 集成 | 2–3 天 |
| M5 | HBBR 混合回归、TLS 集成、压力和安全测试 | 2–3 天 |
| M6 | 双语文档、CI、候选构建、灰度与正式发布 | 2–3 天 |

预计 10–16 个专注工程日，另加生产观察窗口。任何 HBBR 混合协议、客户端版本或上游锚点意外差异都可能触发停止条件，而不是自动扩大范围。

推荐 PR：

1. docs: add patch-v1.1.0 implementation plan
2. test(hbbr): lock the official mixed WSS/native Relay contract
3. feat(config): add schema v2 and WebSocket Signal configuration
4. feat(ws): add bounded WSS session transport and lifecycle
5. feat(signal): add shared registration and cross-transport routing
6. feat(geo): add WSS Relay health and transport-aware selection
7. test(ws): add TLS, cross-transport and stress coverage
8. docs(release): document deployment, migration, rollback and bump PATCH_VERSION

每个 PR 必须保持已发布 patch-v1.0.0 的 17 个 overlay 测试及构建矩阵继续通过。

## 22. 灰度部署

### 阶段 A：隔离验证

- 新测试域名；
- 独立 HBBS 数据目录副本或测试身份；
- 一个同时开放 21117 与 443 /ws/relay 的官方 HBBR；
- 两台测试客户端；
- 不修改生产客户端配置。

### 阶段 B：生产旁路

- 在中心为 WSS Signal 准备受控入口；
- websocket_signal.enabled 先保持 false；
- 校验证书、可信代理、防火墙和所有 Relay endpoint；
- 启用后只允许指定测试客户端打开 WebSocket；
- 原生客户端继续使用现网路径。

### 阶段 C：七 Relay 验收

- 每个节点分别验证证书和 /ws/relay；
- 每个节点分别完成真实 WSS 或混合控制；
- 人工制造一个节点 WSS 故障，验证 Geo 回退；
- 记录每个节点实际镜像 digest、证书 SAN 和验收时间。

### 阶段 D：正式开放

- 服务端 WSS 保持启用；
- 客户端仍默认关闭；
- 企业网络用户按需开启；
- 观察 HBBS session、HBBR 带宽、失败率和 Geo fallback。

## 23. 回滚

功能回滚：

1. 将 websocket_signal.enabled 改为 false。
2. 执行 reload-starry-config。
3. 等待或执行 WSS drain。
4. 验证原生 Secure TCP、P2P 和 21117 Relay。

版本回滚：

1. 固定回上一个已验证的 1.1.16-patch-v1.0.0 镜像 digest。
2. 同时恢复 config.v1.backup.yaml。
3. 不修改 HBBS/HBBR 身份密钥和数据库。
4. 保留 Nginx WSS 片段无害，但企业 WSS 客户端会失去注册能力；必须明确通知其临时不可用。
5. 用原生双客户端验证 P2P、Secure TCP 和 Relay。

禁止只回滚镜像而继续使用 v2 配置后宣称 Geo/Secure TCP 正常。

## 24. Definition of Done

- [ ] WSS RegisterPk 后客户端稳定显示 Ready，心跳持续。
- [ ] Ready 之后完成两台真实客户端桌面控制。
- [ ] WSS ↔ WSS 完整通过。
- [ ] WSS → native 和 native → WSS 完整通过。
- [ ] 同一客户端两种模式切换没有幽灵 route。
- [ ] API 登录与未登录路径均通过。
- [ ] WSS 不进入 Secure TCP；原生 Secure TCP 无回归。
- [ ] 原生 P2P 优先级和性能无明显回退。
- [ ] 七个生产 Relay 的 /ws/relay 和混合 Relay 分别验收。
- [ ] WSS 故障节点不会被 Geo 分配给 WSS/Mixed 会话。
- [ ] 可信代理、Origin、帧上限、队列背压和限速测试通过。
- [ ] 日志和 Artifact 不含 Token、私钥、完整 Peer ID 或原始 payload。
- [ ] 配置 v1 兼容、v2 校验和 v1 回滚演练通过。
- [ ] Overlay 在干净上游应用两次并通过 diff --check。
- [ ] 全部单元、集成、双架构、Windows、DEB 和容器测试通过。
- [ ] GitHub Release、SHA256SUMS、SBOM 和 GHCR manifest 完整。
- [ ] 发布后通过共同 Relay UUID 和 HBBR 日志证明真实数据通道。

## 25. 必须停止并报告的条件

遇到以下任一情况立即停止当前里程碑，记录错误、原因和建议，不自行扩大范围：

1. overlay 新锚点在固定上游中匹配数量不是 1。
2. 官方 HBBR 的自动化混合传输测试失败。
3. RustDesk 目标客户端版本的 WSS 首帧或心跳与固定源码不一致。
4. 无法在不削弱 RegisterPk 校验的前提下共享身份逻辑。
5. WSS 与原生切换出现无法通过 generation 消除的误路由。
6. WSS 健康选择必须修改 HBBR 协议才能工作。
7. TLS 证书、反向代理或七节点入口未准备好，无法完成真实验收。
8. 任何建议要求复制 Pro 闭源代码、跳过 TLS 或打印密钥/Token。

需要 fork HBBR、修改客户端、改变 Geo 语法或扩展 API 鉴权时，必须另立方案并取得明确授权。

## 26. 参考基线

- [当前 Starry overlay 注入脚本](scripts/apply_overlay.py)
- [当前外部配置示例](config/config.example.yaml)
- [当前 Secure TCP 实现](overlay/src/secure_tcp.rs)
- [当前 Geo Relay 实现](overlay/src/geo_relay.rs)
- [官方 rustdesk-server 1.1.16 HBBS](https://github.com/rustdesk/rustdesk-server/blob/1.1.16/src/rendezvous_server.rs)
- [官方 rustdesk-server 1.1.16 HBBR](https://github.com/rustdesk/rustdesk-server/blob/1.1.16/src/relay_server.rs)
- [RustDesk 1.4.9 持久信令客户端](https://github.com/rustdesk/rustdesk/blob/1.4.9/src/rendezvous_mediator.rs)
- [RustDesk 1.4.9 WSS 跳过 Secure TCP](https://github.com/rustdesk/rustdesk/blob/1.4.9/src/common.rs)
- [hbb_common WebSocket 地址转换](https://github.com/rustdesk/hbb_common/blob/69cea8dafee147848ae88702029f4bf7df7224c3/src/websocket.rs)
- [RustDesk allow-websocket 官方说明](https://rustdesk.com/docs/en/self-host/client-configuration/advanced-settings/#allow-websocket)
- [RustDesk /ws/id 与 /ws/relay 官方部署说明](https://rustdesk.com/docs/en/self-host/rustdesk-server-pro/faq/#8-add-websocket-secure-wss-support-for-the-id-server-and-relay-server-to-enable-secure-communication-for-all-platforms)
- [BetterDesk 公开 WSS Signal 结构参考](https://github.com/UNITRONIX/BetterDesk/blob/main/betterdesk-server/signal/ws.go)

## 27. 最终实施顺序

唯一推荐主线：

1. 固定 1.1.16 与客户端协议 fixture。
2. 先证明官方 HBBR 的 WSS ↔ native 混合 Relay。
3. 实现 schema v2 和有界 WsWriteTransport。
4. 实现 RegisterPk 共享校验、持久 session 和 generation 清理。
5. 打通 WSS 与原生的双向信令。
6. 增加 WSS Relay 健康检查和 Geo Requirement。
7. 完成 TLS、压力、安全和真实客户端矩阵。
8. 先隔离灰度，再逐一验收七个 Relay。
9. 最后修改 PATCH_VERSION 为 1.1.0 并发布 1.1.16-patch-v1.1.0。

这条顺序首先验证最可能改变项目边界的 HBBR 混合能力，再进入 HBBS 实现，可以避免完成大量代码后才发现必须 fork HBBR。
