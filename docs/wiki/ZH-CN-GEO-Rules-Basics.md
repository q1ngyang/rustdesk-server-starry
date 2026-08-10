# Geo 规则：入门

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/GEO-Rules-Basics) | **简体中文**

Geo 规则让 HBBS 根据它观测到的两端客户端公网地址，从有序 HBBR 列表中选择目标。
它不会重定向 API 流量、修改客户端显示位置或改造 HBBR。先用一条宽泛规则和两台
Relay 建立可观察的结果；真实会话验证通过后再增加复杂度。

## 一分钟理解选择过程

连接需要 Relay 时，Starry 会：

1. 判断传输要求（`native`、`wss` 或 `mixed`）；
2. 生成当前符合要求的 Relay 列表；
3. 查询两端观测公网地址的 Geo 信息；
4. 从上到下匹配规则；
5. 在第一条可用的命中规则内，按列表顺序选择第一个符合要求的 Relay；
6. 若没有 Geo 规则能选出 Relay，则在符合要求的 Relay 中恢复官方轮询选择。

如果规则匹配、但该规则列出的 Relay 全部不可用，会继续尝试后续规则。具体策略应
放在前面，宽泛兜底应放在最后。

## 1. 声明 Relay 池

```yaml
version: 1

relay_servers:
  - relay-asia-1.example.com:21117
  - relay-asia-2.example.com:21117
  - relay-us-1.example.com:21117
```

规则使用的每台 Relay 都必须出现在此列表。HBBR 可以位于中心主机或独立节点，
但地址必须能被客户端访问，并使用同一个 HBBS 公钥。

## 2. 只准备规则所需的 MMDB

仅国家规则可以使用 Country 或 City MMDB：

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
    url: ""
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: ""
```

请将示例 URL 换成合法、可信的数据源，或留空并自行把文件放到配置路径。镜像不
内置数据库。城市字段需要 City 数据库；ASN 和 ISP 字段需要 ASN 数据库。

## 3. 编写第一组规则

```yaml
geo:
  enabled: true
  rules:
    - name: East Asia to Asia Relay
      symmetric: true
      match:
        client_a: CN/JP/KR/TW
        client_b: "*"
      relays:
        - relay-asia-1.example.com:21117
        - relay-asia-2.example.com:21117

    - name: Default
      symmetric: true
      match:
        client_a: "*"
        client_b: "*"
      relays:
        - relay-us-1.example.com:21117
```

含义是：

- 任一客户端位于 `CN`、`JP`、`KR` 或 `TW` 时，优先
  `relay-asia-1`，不可用后才转到 `relay-asia-2`；
- 其余情况在符合要求时使用 `relay-us-1`。

这里的 `symmetric: true` 很重要：发起连接的一端并不是定义地理位置的稳定方式。
无论区域客户端是 A 端还是 B 端，规则都能匹配。

## 表达式基础

| 语法 | 含义 | 示例 |
| --- | --- | --- |
| `*` | 任意地址，包括缺少 MMDB 信息的地址 | `client_b: "*"` |
| `/` | 或 | `CN/JP/KR` |
| `+` | 且；优先级高于“或” | `country:CN+asn:AS4134` |
| `( )` | 分组 | `(CN/JP)+continent:AS` |
| `XX` | 两位国家码简写 | `CN` 等于 `country:CN` |
| `field:value` | 匹配一个明确字段 | `city:Shanghai` |

YAML 中包含 `*` 的字符串必须加引号；值可能带标点时也应引用整个表达式。Geo 比较
不区分大小写。ISP 是不区分大小写的子串匹配；其余文本条件与 MMDB 值进行不区分
大小写的精确匹配。

支持的字段：

| 字段 | 示例 | 数据库 |
| --- | --- | --- |
| `continent` | `continent:AS` | Country 或 City |
| `country` | `country:US` | Country 或 City |
| `subdivision`、`region` | `region:CA` | City |
| `city` | `city:Shanghai` | City |
| `geoname`、`city_id` | `geoname:1796236` | City |
| `asn` | `asn:AS4134` 或 `asn:4134` | ASN |
| `isp`、`asn_org` | `isp:'China Telecom'` | ASN |

GeoNames ID 和 ASN 必须是正整数。城市和行政区名称取决于 MMDB 内实际提供的名称。
代码或数字 ID 通常比翻译后的名称更少歧义。

## 4. 重载并检查

使用项目 Compose 示例时：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115"

docker logs --tail 100 rustdesk-starry-hbbs
```

应看到 Starry 配置被接受、Geo 规则数量和已读取的数据库路径；规则需要的数据库不应
出现缺失警告。

确认 Relay 池：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"
```

管理端口按设计仅限回环地址。请在容器或 HBBS 网络命名空间内执行命令，绝不能将
管理命令接口公开或反向代理到公网。

## 5. 用真实公网地址预览

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 192.0.2.10 198.51.100.20 native\n' | nc -w 2 127.0.0.1 21115"
```

将文档保留地址替换为 HBBS 实际观测到的公网源地址。两个设备共用一个 NAT 时，
两个参数都填写该公网地址。输出是 Rust 调试字符串形式的已选 Relay；若没有符合
要求的 Relay 则为 `""`。

`test-geo` 只是决策预览：它不会注册客户端、打开 HBBR、证明 DNS/防火墙有效，也
不会测量延迟。

## 6. 完成真实测试

使用两台 RustDesk 客户端，并把客户端“中继服务器”字段留空。确认：

1. 两端都注册到当前 HBBS，并使用预期公钥；
2. 强制 Relay 或自然进入 Relay 的会话抵达预期 HBBR；
3. 桌面、输入和合适时长的持续传输可用；
4. 停止第一优先 Relay 后，只有 HBBS 观察到它不可用，才选择下一台有序 Relay。

记录两端客户端时间戳及对应 HBBS/HBBR 日志。在判定策略可用于生产前，继续执行
[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。

## 新手常见错误

| 现象 | 常见原因 |
| --- | --- |
| 总是选择同一台 Relay | 这可能完全正确：Relay 顺序是严格优先级，不是负载均衡。 |
| 兜底规则遮住区域规则 | 兜底规则位置过早，应移到最后。 |
| 国家匹配正常，城市始终不匹配 | City MMDB 缺失、不可读，或不包含预期名称。 |
| 服务端策略像是被忽略 | 客户端“中继服务器”字段非空。 |
| MMDB URL 下载到 HTML 页面 | 数据源要求认证或跳到许可证页面；应改用获授权的文件直链。 |
| `test-geo` 正常但无法远控 | 只证明了 Geo 选择，没有证明 HBBR 可达和会话流。继续验证下一层。 |
