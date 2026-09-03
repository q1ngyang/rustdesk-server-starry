# 配置参数详解

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference) | **简体中文**

Starry 从 HBBS 数据目录下的 `starry/config.yaml` 读取配置。容器中的路径是
`/root/starry/config.yaml`，因为 `/root` 是持久化数据挂载点。HBBS 首次启动还会
生成 `starry/config.example.yaml` 作为本地参考。

解析器会拒绝未知字段、重复列表项、超范围数值，以及未在 `relay_servers` 声明的中继
服务器。首次加载时，如果文件不存在、为空或无效，Starry 不会只启用其中一部分：HBBS
会记录错误并保持上游兼容行为。已经有有效配置后，新配置加载失败会完整保留最近一次
有效配置。这些机制用于避免半生效状态，但仍必须检查日志和启用结果。

## 中文术语说明

文档正文优先使用中文常用说法；配置值、接口字段和日志原文保留英文，便于对照：

| 配置或日志用词 | 本文含义 |
| --- | --- |
| `version` / schema | 配置结构版本 |
| generation | 配置代次；每次成功启用配置后递增 |
| digest | 配置内容摘要，用于确认各组件采用同一内容 |
| last-known-good | 最近一次成功启用的有效配置 |
| activation acknowledgement | 配置启用确认 |
| endpoint | 服务地址或健康检查地址 |
| `audit` | 仅记录认证结果，不拦截连接 |
| `enforce` | 强制认证，拒绝不符合要求的连接 |
| mixed | 一端使用 WSS、另一端使用原生连接的混合方式 |
| Control Agent | Starry 管理代理；不是账户 API 服务 |

出现配置项时必须填写表格左侧的精确英文值，不能把中文说明写入 YAML。

## 配置结构版本和功能范围

| 字段 | 必填 | 可用值 | 含义 |
| --- | --- | --- | --- |
| `version` | 是 | `1`、`2`、`3`、`4`、`5` | 配置结构版本。只有 patch-v1.3.1 FastMedia 策略使用 `5`。 |

结构版本 `1` 支持 Relay、Secure TCP、MMDB 和 Geo，并拒绝 `websocket_signal` 与
`connection_auth`；版本 `2` 增加可选 WebSocket Signal，但仍拒绝 `connection_auth`；
版本 `3` 新增连接认证并拒绝 `relay_quality`；版本 `4` 新增可选的 Akari Relay
质量选择与 FastCompat Relay 授权；版本 `5` 新增 FastMediaV1 策略与 Relay UDP endpoint，
不改变 schema 4 或 Relay Quality v1 语义。顶层和嵌套未知字段均会被拒绝，避免拼写错误悄悄
改变部署结果。

## 中继服务器列表：`relay_servers`

```yaml
relay_servers:
  - relay-asia-1.example.com:21117
  - relay-us-1.example.com:21117
```

这是 Starry HBBS 可以分配的完整中继服务器列表。值会去除首尾空白，不能为空，并且按
不区分大小写的方式保持唯一。

- Geo 规则引用的每个 Relay 都必须出现在这里；
- 启用 WebSocket Signal 时，`relay_health.endpoints` 必须恰好覆盖此列表，每个
  Relay 一个 WSS 端点；
- 主机名和端口表示原生 HBBR 目标，不是 HTTP URL；
- 若需由 HBBS 执行 Geo 分配，RustDesk 客户端的“中继服务器”字段应留空。客户端
  指定的 Relay 会覆盖服务端分配。

## 安全 TCP：`secure_tcp`

```yaml
secure_tcp:
  mode: auto
  handshake_timeout_ms: 18000
  idle_timeout_ms: 30000
  max_frame_bytes: 65536
```

| 字段 | 默认值 | 有效范围或值 | 说明 |
| --- | ---: | --- | --- |
| `mode` | `off` | `off`、`auto` | `auto` 协商兼容客户端的原生加密信令，同时仍接受有效的明文首帧。 |
| `handshake_timeout_ms` | `18000` | `1000..120000` | 协商最长时间。 |
| `idle_timeout_ms` | `30000` | `1000..600000` | 已协商传输的空闲读取超时。 |
| `max_frame_bytes` | `65536` | `4096..16777216` | 最大安全帧；仅在有测量依据时调高。 |

Secure TCP 作用于 `21116/TCP` 上的原生 HBBS 信令。它不会自行加密或代理 HBBR，
也不同于 API 使用的 HTTPS。

## 地理位置数据库：`mmdb`

```yaml
mmdb:
  update_interval_hours: 168
  update_on_start: true
  force_update: false
  download_timeout_seconds: 600
  minimum_bytes: 65536
  country:
    path: mmdb/GeoLite2-Country.mmdb
    url: https://downloads.example.com/GeoLite2-Country.mmdb
  city:
    path: mmdb/GeoLite2-City.mmdb
    url: https://downloads.example.com/GeoLite2-City.mmdb
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: https://downloads.example.com/GeoLite2-ASN.mmdb
```

| 字段 | 默认值 | 有效范围 | 含义 |
| --- | ---: | --- | --- |
| `update_interval_hours` | `168` | `0..8760` | 定期刷新间隔；`0` 关闭定期刷新。 |
| `update_on_start` | `true` | 布尔值 | 启动时检查已配置的下载 URL。 |
| `force_update` | `false` | 布尔值 | 即使现有文件仍在间隔内也下载。只应临时开启，完成后关闭。 |
| `download_timeout_seconds` | `600` | `1..3600` | 单次下载超时。 |
| `minimum_bytes` | `65536` | `1024..1073741824` | 拒绝明显过小的下载；它不等同于许可证或真实性校验。 |

`country`、`city` 和 `asn` 各有以下字段：

| 字段 | 默认值 | 规则 |
| --- | --- | --- |
| `path` | `mmdb/GeoLite2-Country.mmdb`、`mmdb/GeoLite2-City.mmdb` 或 `mmdb/GeoLite2-ASN.mmdb` | 必须是无路径穿越的相对 `mmdb/*.mmdb` 路径；替换前会拒绝符号链接路径组件。 |
| `url` | 空 | 可选的 `https://` 下载地址。为空表示自行管理本地文件；不接受重定向。请使用有权使用的数据源。 |

下载先写入临时文件，检查最小大小、MaxMind 标记和读取兼容性后，再原子替换目标；
超过 1 GiB 的响应会在替换前被拒绝。失败时会保留上一个可读文件。镜像不包含
GeoLite2 数据，也不提供数据库许可证。

只需准备规则实际使用的数据库：

| 规则字段 | 所需数据库 |
| --- | --- |
| `continent`、`country` 或简写国家码 | Country **或** City |
| `subdivision`、`region`、`city`、`geoname`、`city_id` | City |
| `asn`、`isp`、`asn_org` | ASN |

## 地理位置规则：`geo`

```yaml
geo:
  enabled: true
  rules:
    - name: Asia preference
      symmetric: true
      match:
        client_a: CN/JP/KR
        client_b: "*"
      relays:
        - relay-asia-1.example.com:21117
        - relay-asia-2.example.com:21117
```

| 字段 | 默认值 | 规则 |
| --- | ---: | --- |
| `enabled` | `false` | 启用时至少需要一条规则和一个 `relay_servers` 条目。 |
| `rules[].name` | 无 | 必填、非空且唯一。 |
| `rules[].symmetric` | `true` | 为 true 时，还会交换 A/B 两端再尝试匹配。 |
| `rules[].match.client_a` | `*` | 第一端观测到的公网地址表达式。 |
| `rules[].match.client_b` | `*` | 第二端观测到的公网地址表达式。 |
| `rules[].relays` | 无 | 必填、有序且唯一；每项必须位于 `relay_servers`。 |

规则从上到下求值。命中一条规则后，其 Relay 是严格优先级：选择当前可用的第一个。
继续阅读 [Geo 规则：入门](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics)
和 [Geo 规则：进阶](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Advanced)。

## WebSocket 信令：`websocket_signal`

此部分要求 `version: 2`、`3`、`4` 或 `5`，并且必须显式启用。

```yaml
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
      - relay: relay-asia-1.example.com:21117
        url: wss://relay-asia-1.example.com/ws/telemetry
        telemetry_secret_file: /run/secrets/starry-relay-telemetry
        fast_media_udp_port: 21119
```

### 会话和资源限制

| 字段 | 默认值 | 有效范围 |
| --- | ---: | --- |
| `enabled` | `false` | 布尔值 |
| `registration_timeout_ms` | `10000` | `1000..120000` |
| `keepalive_interval_ms` | `12000` | `1000..300000`，且小于 `idle_timeout_ms` |
| `idle_timeout_ms` | `45000` | `2000..600000` |
| `max_frame_bytes` | `65536` | `4096..16777216` |
| `outbound_queue_capacity` | `64` | `1..4096` |
| `max_sessions` | `10000` | `1..1000000` |
| `max_sessions_per_effective_ip` | `512` | `1..max_sessions` |
| `registration_rate_per_minute` | `300` | `1..100000` |

这些限制用于保护 HBBS 资源，不能替代防火墙、反向代理和主机监控。

### 代理身份和 Origin

`trusted_proxies` 是允许信任其转发客户端 IP 请求头的唯一 CIDR 列表。默认仅信任
`127.0.0.1/32` 和 `::1/128`，适合 Nginx 与 HBBS 共用主机网络的情况。只有确认
HBBS 实际看到的源地址后，才能加入 Docker 网桥或外部代理网段。不要为了让请求头
生效而使用 `0.0.0.0/0`。

`allowed_origins` 是可选的精确 `http://` 或 `https://` Origin 列表，不得包含路径、
凭据、查询或片段。不发送 `Origin` 的原生 RustDesk 客户端仍可接入；发送 Origin 的
客户端必须精确匹配其中一项，空列表会拒绝所有携带 Origin 的请求。

### `relay_health`

| 字段 | 默认值 | 有效范围或规则 |
| --- | ---: | --- |
| `interval_seconds` | `60` | `5..3600` |
| `timeout_ms` | `5000` | `500..120000` |
| `success_threshold` | `1` | `1..100` 次连续成功 |
| `failure_threshold` | `2` | `1..100` 次连续失败 |
| `endpoints[].relay` | 无 | 必填、唯一，并等于某个 `relay_servers` 条目。 |
| `endpoints[].url` | 无 | 必填且唯一；必须是 `wss://`、DNS 主机名和精确 `/ws/relay`（仅 legacy health）或 `/ws/telemetry` 路径，不得有凭据、查询或片段。 |
| `endpoints[].telemetry_secret_file` | 无 | 绝对 secret-file 路径；`/ws/telemetry` 必填，`/ws/relay` 禁止；文件内容不会序列化。 |
| `endpoints[].fast_media_udp_port` | 无 | 仅 schema v5，`1..65535`；只能与认证 `/ws/telemetry` 一起使用。声明 endpoint 不等于 listener 已健康。 |

当 `enabled: true` 时，端点的 Relay 名称必须恰好覆盖 `relay_servers`。健康探测验证
分配所需的 WSS/TLS 路径，但不能替代两台客户端的实际远控测试。参见
[反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)。

## 连接认证：`connection_auth`

本节要求 `version: 3`、`4` 或 `5`，用于控制原生 TCP、安全 TCP、WSS 上控制端发出的
`PunchHoleRequest` 与直接 `RequestRelay`。UDP 不支持发起这种已认证连接，也不会分配
中继服务器。

```yaml
connection_auth:
  mode: audit
  issuer: https://kessoku.example
  audience: rustdesk-connect
  token_use: access
  required_scope: connect:initiate
  max_token_bytes: 8192
  clock_skew_seconds: 30
  jwks:
    file: /var/lib/starry-auth/jwks.json
    url: https://kessoku.example/api/internal/v1/auth/jwks
    refresh_interval_seconds: 300
    max_stale_seconds: 3600
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
  introspection:
    required: true
    url: https://kessoku.example/api/internal/v1/auth/introspect
    timeout_ms: 1000
    positive_cache_seconds: 10
    negative_cache_seconds: 1
    max_cache_entries: 100000
    ca_file: /run/secrets/starry-auth/internal-ca.pem
    cert_file: /run/secrets/starry-auth/hbbs-client.pem
    key_file: /run/secrets/starry-auth/hbbs-client-key.pem
    server_name: kessoku.example
```

| 字段 | 默认值 | 规则 |
| --- | --- | --- |
| `mode` | `off` | `off`、`audit`、`enforce`；`audit` 记录但继续。部署层 `--must-login` floor 强制 effective enforce。 |
| `issuer` | 空 | audit/enforce 必填 HTTPS issuer，精确匹配 `iss`。 |
| `audience` | 空 | audit/enforce 必填，必须存在于 `aud`。 |
| `token_use` | `access` | 精确要求的 `token_use` claim。 |
| `required_scope` | `connect:initiate` | 完整 scope 值，不做 substring 匹配。 |
| `max_token_bytes` | `8192` | `128..8192`，JWT parse 前检查。 |
| `clock_skew_seconds` | `30` | `0..300`，用于 `iat`、`nbf`、`exp`。 |
| `jwks.file` | 空 | 本地公共 Ed25519 JWKS 与持久 cache path；enforce 要求非空初始文件。 |
| `jwks.url` | 空 | 可选内部 HTTPS refresh URL；存在时 CA/cert/key/server-name 全部必填，且 URL host 必须等于 `server_name`。 |
| `jwks.refresh_interval_seconds` | `300` | `30..86400`。 |
| `jwks.max_stale_seconds` | `3600` | `30..604800`；超过后 fail closed。 |
| `jwks.ca_file` / `cert_file` / `key_file` / `server_name` | 空 | JWKS refresh 的 TLS 1.3-only 精确信任 CA 与客户端身份；禁用系统根证书。 |
| `introspection.required` | `false` | true 时缺少 client 为无效配置；只要 client 已配置，请求错误无论此 flag 都 fail closed。 |
| `introspection.url` | 空 | 只允许 TLS 1.3 HTTPS；存在时 CA/cert/key/server-name 全部必填，禁用系统根证书，且 URL host 必须等于 `server_name`。 |
| `introspection.timeout_ms` | `1000` | `100..10000`；只对 server error 限制性重试一次。 |
| `positive_cache_seconds` | `10` | `1..60`，且不超过 token expiry。 |
| `negative_cache_seconds` | `1` | `0..1`。 |
| `max_cache_entries` | `100000` | `1..1000000`，确定性淘汰最旧 entry。 |

只接受带唯一显式 `kid` 的 EdDSA/Ed25519 公共 JWK；拒绝私有、对称或重复 key material。
raw token 不会作为 cache key 或 status label。进入 audit/enforce 前先阅读
[连接认证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Connection-Authentication)。

## Relay 质量：`relay_quality`

此冻结 Akari 扩展要求 `version: 4` 或 `5`，默认关闭。官方客户端不会声明能力，仍使用传统的
单 Relay 分配。

```yaml
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
```

| 字段 | 默认值 | 有效范围或规则 |
| --- | ---: | --- |
| `enabled` | `false` | 启用时至少需要两个非 legacy 质量 Relay、完整 health endpoint 覆盖，并要求 `max_candidates >= 2`。 |
| `strategy` | `adaptive` | `adaptive` 先探测 GEO primary，仅在不佳时扩展；`eager` 立即探测全部候选。 |
| `legacy_fallback_relays` | `[]` | `relay_servers` 的唯一子集；只作显式普通 fallback，绝不进入质量候选。 |
| `max_candidates` | `3` | 关闭时 `1..5`，启用时 `2..5`。 |
| `primary_probe_samples` | `3` | `1..20` 且不大于 `probe_samples`；GEO primary 的顺序样本数。 |
| `primary_accept_score` | `8000` | `1..10000`；只由 HBBS 解释。 |
| `primary_max_loss_basis_points` | `500` | `0..10000`；任一可用端点超过阈值即触发扩展。 |
| `p2p_probe_grace_ms` | `300` | `0..5000`；让成功 P2P 在主动探测前取消 allocation。 |
| `probe_samples` | `5` | 每个 Relay、每个端点尝试 `3..20` 次。 |
| `probe_interval_ms` | `50` | `20..2000`。 |
| `probe_timeout_ms` | `1000` | `100..5000`；单样本硬超时。不同候选并发，同候选样本按顺序执行。 |
| `report_timeout_ms` | `15000` | `1000..60000`；服务端强制总 deadline。adaptive 必须容纳两段 primary window、一段并发 expansion window 和 1000 ms 信令余量；eager 必须容纳两个完整 window。 |
| `max_telemetry_age_seconds` | `180` | `5..3600` 且不小于 health interval 加 timeout；更旧 load 会排除候选。 |
| `allocation_ttl_seconds` | `30` | `5..300` 且大于 `report_timeout_ms`；只用于清理，不决定报告有效性。 |
| `cache_ttl_seconds` | `300` | `30..86400`；用于对称 `/24` 或 `/56` 网段对选择。 |
| `max_allocations` | `10000` | `100..1000000`；分别作为待处理 allocation、decision 与网段缓存 map 的硬上限，超限时先淘汰最旧项。 |
| `hysteresis_basis_points` | `500` | `0..5000`；新分数未超过此差值时保留缓存 Relay。 |
| `missing_report_penalty_basis_points` | `1000` | 每个缺失端点测量惩罚 `0..10000`。 |
| `rtt_bad_ms` | `300` | `10..10000`；RTT 归一化上限。 |
| `jitter_bad_ms` | `100` | `1..5000`；jitter 归一化上限。 |
| `weights` | `4000/2000/2500/1500` | RTT/jitter/loss/load 均须大于 0，合计必须为 `10000`。 |

每个非 legacy 质量 Relay 必须有一个唯一、受认证的 `/ws/telemetry` endpoint 和绝对
`telemetry_secret_file`，URL 也必须唯一；`/ws/relay` 只允许用于显式 legacy
health/fallback。启用 WebSocket Signal 时仍保留原有覆盖全部 Relay 的严格规则。候选与报告
只在 HBBS 信令内传递。Kessoku 可以管理配置并读取 Control API 计数，但
Akari 和 HBBR 都不会连接 Control Agent。每台 HBBR 应设置
`STARRY_RELAY_MAX_SESSIONS`，该值现在是真实 admission 上限；`TOTAL_BANDWIDTH` 仍以
Mbit/s 表示容量。HBBR 通过 `STARRY_RELAY_TELEMETRY_SECRET_FILE` 读取同一密钥。内部
mTLS 优先；反向代理终止 TLS 时再由 secret-file HMAC 提供端到端请求/响应认证。HBBS 的
load 评分只信任自己拉取并验签的遥测。启用 Relay 质量时，即使
`websocket_signal.enabled` 为 false，经过证书验证的探测仍会运行。HBBR 必须显式声明
probe/load protocol v1，不能从版本字符串推断；遥测缺失、不完整或过期时，该 Relay 会
退出质量 offer，但显式配置的普通 fallback 仍可使用。

公开 `/ws/relay` 握手和 `RelayProbeResponse` 不再包含详细 load。HBBR 默认每传输源 IP
每分钟 120 次、全局 10,000 次 probe，可用 `STARRY_RELAY_PROBE_PER_IP_PER_MINUTE` 和
`STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE` 调整。`STARRY_RELAY_DRAINING=true` 或
`STARRY_RELAY_DRAINING_FILE` 存在时只拒绝新配对，已有会话继续。详见
[Relay Telemetry v1](../../reference/RELAY-TELEMETRY-v1.zh-CN.md)。

## 极速模式：`fast_mode.relay`

schema v4 只支持 FastCompat。schema v5 保留该可靠路径，并增加独立 FastMediaV1
Relay UDP 策略。两个开关都默认 false；客户端不能选择签名 Relay，任一 UDP 故障都保留
普通 HBBR 会话。

```yaml
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    authorization_ttl_seconds: 90
    max_bitrate_kbps: 50000
    relay_max_datagram: 1200
```

| 字段 | 默认值 | 有效范围或规则 |
| --- | ---: | --- |
| `fast_compat_enabled` | `false` | 要求连接鉴权为 `audit` 或 `enforce`，并启用 `secure_tcp.mode: auto` 或 WebSocket 信令。Relay Quality 有 decision 时保持权威，否则 HBBS 只签普通 GEO/failover 最终选择。 |
| `fast_media_v1_enabled` | `false` | 仅 schema v5。除 FastCompat 安全门禁外，还要求至少一个可选 Relay 具有新鲜认证 telemetry schema 2、明确 `fast_media_relay_udp = 1`、声明 UDP port 且 listener 健康。 |
| `authorization_ttl_seconds` | `90` | `30..300`；功能关闭时也会校验，重试不延长有效期。 |
| `max_bitrate_kbps` | `50000` | `1000..200000`；签名编码源上限，HBBR wire allowance 不超过 `ceil(source × 1.45)`。 |
| `relay_max_datagram` | `1200` | 仅 schema v5，`608..1400`；含 32 字节 AKR1 header 的完整 UDP payload。 |

HBBS 只在鉴权返回严格 `allow` 且普通最终 Relay 已固定后签名；存在 Relay Quality
decision 时，两者必须完全一致，否则使用服务端普通 GEO/failover 选择。FastMedia 分别
签发角色 1 controller 与角色 2 target，双端在 HBBR 绑定后才开始；旧六字段
FastCompat 继续兼容。任一门禁缺失都不签 FastMedia，普通 Relay 继续。官方客户端忽略
tag 64。

若 WSS 在反向代理终止 TLS，必须禁止公网直接访问 HBBS 明文 WebSocket 监听端口。线协议、
重放、资源和隐私要求见
[极速 Relay 授权协议 v1](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/reference/FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)和
[FastMedia Relay UDP v1](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/reference/FAST-MEDIA-RELAY-UDP-v1.zh-CN.md)。

## 重新加载配置时的行为

未部署 Control Agent 的首次接入可在修改后重启 HBBS：

```sh
docker restart rustdesk-starry-hbbs
```

后续受管理的变更应使用已认证的版本化管理接口先预览再应用，或调用
`POST /control/v1/runtime:reload`。完整有效的新配置必须得到所有相关子系统确认，之后才
会作为新代次一次性启用。空配置、无效配置或被拒绝的重新加载会保留此前的有效配置、
内容摘要、中继服务器和认证状态，并设置 `last_error`。若从未加载过有效配置，HBBS
保持上游兼容行为。修正或恢复磁盘文件后必须再次加载并确认成功；进程仍在运行不代表
新配置已经生效。

## 可直接修改的配置模板

- [`config.single-host.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.single-host.yaml)：单机完整接入模板；地理位置规则和 WebSocket 默认关闭，准备好依赖后再启用；
- [`config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml)：仅 Secure TCP；
- [`config.geo-basic.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-basic.yaml)：Geo 入门策略；
- [`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml)：嵌套和方向敏感规则；
- [`config.websocket.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.websocket.yaml)：WebSocket Signal；
- [`config.auth-audit.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.auth-audit.yaml)：schema v3 连接认证 audit canary；
- [`config.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.example.yaml)：所有配置板块。

必须替换示例域名和 URL，再通过日志和真实会话验证。
