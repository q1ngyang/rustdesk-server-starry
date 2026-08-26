# Geo 规则：进阶

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Advanced) | **简体中文**

请在国家级规则和真实 Relay 会话验证通过后再使用本页。进阶规则会增加优先级、方向、
ASN/ISP 条件、传输感知可用性和有意设计的回退。只有能用已知公网地址逐分支测试时，
复杂度才有价值。

## 语法和优先级

表达式语法是：

```text
expression := and-term ("/" and-term)*
and-term   := primary ("+" primary)*
primary    := predicate | "(" expression ")"
predicate  := "*" | two-letter-country | field:value
```

`+` 的优先级高于 `/`，所以：

```text
CN/JP+asn:AS2516
```

表示 `CN 或（JP 且 AS2516）`，而不是 `（CN 或 JP）且 AS2516`。只要人工审核者
可能产生不同理解，就应加括号。

带引号的值可以包含 `/`、`+`、`(` 或 `)`。单引号和双引号都可使用，反斜杠在
引用值内转义下一个字符：

```yaml
client_a: "city:'A/B'+isp:\"Example (Transit)\""
```

YAML 层和表达式层都会处理引号，因此推荐在双引号 YAML 字符串内使用简单的单引号
字段值。

## 精确匹配与子串匹配

- `continent`、`country`、`subdivision`、`city` 及别名进行不区分大小写的精确匹配；
- `geoname` 和 `asn` 比较正整数；
- `isp`/`asn_org` 对 ASN 组织名称进行不区分大小写的子串搜索。

ISP 标签是数据提供方记录，不是具有约束力的身份。不要在未审计实际 MMDB 记录前，
将某个子串用于安全或计费决策。

## 方向敏感策略

`symmetric: false` 只匹配声明的 A 到 B 方向：

```yaml
- name: China Telecom initiator to US peer
  symmetric: false
  match:
    client_a: "country:CN+asn:AS4134"
    client_b: country:US
  relays:
    - relay-us-1.example.com:21117
```

只有客户端角色确有含义并经过验证时才应使用。地理就近策略通常更适合
`symmetric: true`，因为任一端都可能发起连接。

## 嵌套多条件示例

```yaml
- name: Selected East Asia access networks
  symmetric: true
  match:
    client_a: "((city:Shanghai+isp:'China Telecom')/(city:Seoul+isp:KT))+continent:AS"
    client_b: "country:CN/country:JP/country:KR"
  relays:
    - relay-asia-1.example.com:21117
    - relay-asia-2.example.com:21117
```

A 端需要匹配“上海+运营商”或“首尔+运营商”，并同时属于亚洲；B 端需要位于三个
国家之一。由于规则是对称的，Starry 还会交换两端角色再判断一次。

## 规则和 Relay 排序

例如：

```yaml
rules:
  - name: Specific paid route
    match:
      client_a: "country:CN+asn:AS4134"
      client_b: country:US
    relays:
      - relay-premium.example.com:21117
      - relay-us-1.example.com:21117

  - name: General CN to US
    match:
      client_a: country:CN
      client_b: country:US
    relays:
      - relay-us-1.example.com:21117

  - name: Final catch-all
    match:
      client_a: "*"
      client_b: "*"
    relays:
      - relay-asia-2.example.com:21117
```

关键行为如下：

- 规则按顺序执行，不会自动按“更具体”评分；
- Relay 列表是严格优先级，不是轮询权重；
- 命中规则没有符合要求的 Relay 时，后续规则仍可能选出目标；
- 没有规则选出目标时，HBBS 会在符合当前传输要求的 Relay 中执行官方轮询；
- 一条拥有可用 Relay 的全匹配规则会使后续规则永远无法执行。

## 传输感知可用性

patch v1.1.0 会先按传输要求过滤 Relay，再执行 Geo 排序：

| 要求 | Relay 符合条件的证据 |
| --- | --- |
| `native` | 官方 HBBS/HBBR 原生机制报告在线。 |
| `wss` | 当前配置 generation 的探测已完成，并且证书校验通过的 `/ws/relay` 健康。 |
| `mixed` | 同一台 Relay 同时满足原生在线和 WSS 健康。 |

因此，同一对 IP 在不同传输方式下可能选中不同 Relay。应预览客户端会使用的全部路径：

对 `native`、`wss` 和 `mixed` 分别调用一次已认证的
`POST /control/v1/allocations:simulate`，保持两端地址与 expected generation 不变。

测试 `wss` 和 `mixed` 前先查看健康状态：

先用已认证的 `GET /control/v1/status` 查看 WSS 健康状态。

返回空选择比把客户端分配给不支持所需传输的 Relay 更安全。不要用 ping、普通 HTTPS
或关闭 TLS 校验替代 WSS 证书验证。

## 同 NAT 和数据缺失

同一公网 NAT 后的两个客户端会显示相同公网 IP，测试时将同一地址传入两次。对称规则
行为明确；方向敏感规则不会凭空生成不同 Geo 信息。

若地址在所需 MMDB 中没有记录，对应条件为 false，`*` 仍然匹配。需要这种策略时，
应设置明确兜底规则，并监控数据缺失是否异常增多。

私网、回环、CGNAT 或代理地址不能替代 HBBS 实际观测的公网地址。WebSocket 客户端
必须精确配置 `trusted_proxies`，只接受真实反向代理转发的地址。

## 变更控制方法

进行非简单规则变更时：

1. 保存旧配置及其摘要；
2. 建立测试矩阵：IP 对、方向、传输方式、预期规则、首选 Relay 和故障切换 Relay；
3. 每次只改变一个策略维度；
4. 执行已认证的 Control Agent plan/apply 或 runtime-reload 操作，校验被拒绝就立即停止；
5. 对矩阵每一行执行 `test-geo`；
6. 对生产会使用的每类传输至少验证一次真实会话；
7. 模拟第一优先 Relay 的故障和恢复；
8. 将日志、结果与精确配置版本一并保存。

[`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml)
是完整起点；必须替换每个主机、数据 URL 和策略值。

## 反模式

- 不要把 Geo 规则当访问控制系统；它只分配 Relay；
- 未审计 MMDB 语言值前，不要编写几十个城市名称；
- 不要把始终可用的全匹配规则放在第一条；
- 不要期待 Relay 顺序能够均匀分配负载；
- 不要混用 API 域名、HBBS 域名和 HBBR endpoint；
- 不要把 `test-geo`、HTTP 101 或 Compose 校验当成远控验收完成。
