# 配置参数详解

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Configuration-Reference) | **简体中文**

Starry 从 HBBS 数据目录下的 `starry/config.yaml` 读取配置。容器中的路径是
`/root/starry/config.yaml`，因为 `/root` 是持久化数据挂载点。HBBS 首次启动还会
生成 `starry/config.example.yaml` 作为本地参考。

解析器会拒绝未知字段、重复列表项、超范围数值以及指向未在
`relay_servers` 声明的 Relay。首次加载时，文件不存在、为空或无效不会部分启用
Starry：HBBS 记录错误并保持上游兼容行为。已有有效 generation 后，reload 被拒绝会
完整保留 last-known-good generation。这些是安全设计，但不能因此忽略日志和 activation
ack。

## 文档版本和功能门控

| 字段 | 必填 | 可用值 | 含义 |
| --- | --- | --- | --- |
| `version` | 是 | `1`、`2`、`3` | 配置结构版本。新部署请使用 `3`。 |

结构版本 `1` 支持 Relay、Secure TCP、MMDB 和 Geo，并拒绝 `websocket_signal` 与
`connection_auth`；版本 `2` 增加可选 WebSocket Signal，但仍拒绝 `connection_auth`；
版本 `3` 新增连接认证。顶层和嵌套未知字段均会被拒绝，避免拼写错误悄悄改变部署结果。

## `relay_servers`

```yaml
relay_servers:
  - relay-asia-1.example.com:21117
  - relay-us-1.example.com:21117
```

这是 Starry HBBS 已知的完整 Relay 分配池。值会去除首尾空白，不能为空，并且按
不区分大小写的方式保持唯一。

- Geo 规则引用的每个 Relay 都必须出现在这里；
- 启用 WebSocket Signal 时，`relay_health.endpoints` 必须恰好覆盖此列表，每个
  Relay 一个 WSS 端点；
- 主机名和端口表示原生 HBBR 目标，不是 HTTP URL；
- 若需由 HBBS 执行 Geo 分配，RustDesk 客户端的“中继服务器”字段应留空。客户端
  指定的 Relay 会覆盖服务端分配。

## `secure_tcp`

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

## `mmdb`

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

## `geo`

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

## `websocket_signal`

此部分要求 `version: 2` 或 `3`，并且必须显式启用。

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
        url: wss://relay-asia-1.example.com/ws/relay
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
| `endpoints[].url` | 无 | 必填且唯一；必须是 `wss://`、DNS 主机名和精确 `/ws/relay` 路径，不得有凭据、查询或片段。 |

当 `enabled: true` 时，端点的 Relay 名称必须恰好覆盖 `relay_servers`。健康探测验证
分配所需的 WSS/TLS 路径，但不能替代两台客户端的实际远控测试。参见
[反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)。

## `connection_auth`

本节要求 `version: 3`，门控原生 TCP、Secure TCP、WSS 上控制端
`PunchHoleRequest` 与直接 `RequestRelay`。UDP 发起仍 unsupported 且不分配。

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

## 重载行为

未部署 Control Agent 的首次接入可在修改后重启 HBBS：

```sh
docker restart rustdesk-starry-hbbs
```

后续受管变更应使用已认证的版本化 Control API plan/apply 或
`POST /control/v1/runtime:reload` 流程。激活在 active generation 层原子；完整有效
candidate 由所有必需 subsystem prepare；只有
全部 ack 成功才以新 generation 激活。空/无效/被拒绝的 reload 会保留此前
last-known-good generation、digest、Relay/auth 状态并设置 `last_error`。若从未加载过
有效 generation，HBBS 保持上游兼容行为。修正或恢复磁盘文件并要求成功 activation ack；
进程仍运行不等于成功。

## 可直接修改的配置模板

- [`config.minimal.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.minimal.yaml)：仅 Secure TCP；
- [`config.geo-basic.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-basic.yaml)：Geo 入门策略；
- [`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml)：嵌套和方向敏感规则；
- [`config.websocket.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.websocket.yaml)：WebSocket Signal；
- [`config.auth-audit.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.auth-audit.yaml)：schema v3 连接认证 audit canary；
- [`config.example.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.example.yaml)：所有配置板块。

必须替换示例域名和 URL，再通过日志和真实会话验证。
