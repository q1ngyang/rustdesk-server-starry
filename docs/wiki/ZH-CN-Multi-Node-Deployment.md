# 多节点部署

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Multi-Node-Deployment) | **简体中文**

一个 Starry HBBS 中心需要分配多台官方 HBBR 时使用该拓扑。账户/API 服务可选且保持独立。

## 架构

```mermaid
flowchart LR
    A[客户端 A] -->|注册与信令| S[Starry HBBS 中心]
    B[客户端 B] -->|注册与信令| S
    S -->|选中的 Relay 地址| A
    S -->|选中的 Relay 地址| B
    A <-->|原生 21117 或 WSS /ws/relay| R1[官方 HBBR 1]
    B <-->|原生 21117 或 WSS /ws/relay| R1
    S -. 可选账户层 .-> API[第三方 API]
    S --> R2[官方 HBBR 2]
    S --> R3[官方 HBBR N]
```

HBBS 选择 Relay；HBBR 转发会话；API 两者都不负责。

## 公网名称与端口

仅用于示例的名称：

| 角色 | 名称 | 公网路径 |
| --- | --- | --- |
| 中心 HBBS | `id.example.com` | `21116/TCP+UDP`；可选 `wss://id.example.com/ws/id` |
| API | `api.example.com` | 可选 HTTPS `443/TCP` |
| Relay 1 | `relay-1.example.com` | `21117/TCP`；可选 `wss://relay-1.example.com/ws/relay` |
| Relay 2 | `relay-2.example.com` | 同上 |

每个 WSS URL 必须使用证书覆盖的域名。不要在证书域名失败后改用 IP 或关闭验证。

## 密钥模型

- 中心生成 `id_ed25519` 和 `id_ed25519.pub`。
- 私钥只保留在受保护中心数据和备份中。
- 客户端配置公钥内容。
- 纯 Relay 节点只通过官方 HBBR `KEY` 设置获得相同公钥。
- 需要服务器身份的社区 API 只读挂载 `id_ed25519.pub`。

不要把中心私钥复制到每台 Relay。

## 阶段 1：引导中心

使用：

- [`examples/center/compose.bootstrap.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/center/compose.bootstrap.yaml)
- [`examples/center/.env.example`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/center/.env.example)

```sh
cd /opt/rustdesk-center
cp /path/to/repository/examples/center/.env.example .env
cp /path/to/repository/examples/center/compose.bootstrap.yaml .
mkdir -p data/server data/api

docker compose --env-file .env -f compose.bootstrap.yaml config --quiet
docker compose --env-file .env -f compose.bootstrap.yaml up -d
test -s data/server/id_ed25519
test -s data/server/id_ed25519.pub
```

继续之前先备份身份，并确认客户端能进行一次基础原生 HBBS 注册。

## 阶段 2：准备 Starry Relay 配置

从 `config.geo-basic.yaml` 或 `config.websocket.yaml` 开始。完整候选池写入
`relay_servers`：

```yaml
relay_servers:
  - relay-1.example.com:21117
  - relay-2.example.com:21117
```

每条 Geo 规则只能引用该池中的条目。规则顺序和规则内 Relay 顺序都是严格优先级，不是轮询。

启用 WebSocket Signal 时，`relay_health.endpoints` 必须精确覆盖该池：

```yaml
websocket_signal:
  enabled: true
  relay_health:
    endpoints:
      - relay: relay-1.example.com:21117
        url: wss://relay-1.example.com/ws/relay
      - relay: relay-2.example.com:21117
        url: wss://relay-2.example.com/ws/relay
```

两条精确路径都通过有效 TLS/WebSocket Upgrade，且真实客户端验收已准备好之前不要启用。

## 阶段 3：部署每台纯 Relay

使用：

- [`examples/relay/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/relay/compose.yaml)
- [`examples/relay/.env.example`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/relay/.env.example)

每台 Relay 执行：

```sh
mkdir -p /opt/rustdesk-relay/data
cd /opt/rustdesk-relay
cp /path/to/repository/examples/relay/compose.yaml .
cp /path/to/repository/examples/relay/.env.example .env
```

把中心公钥单行内容填入 `RUSTDESK_PUBLIC_KEY`，然后：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml logs --tail 100 hbbr
```

该示例中的可选 Relay 调优值属于官方 HBBR 1.1.16，而不是 Starry 覆盖层：

| 环境变量 | 示例值 | 单位与作用 |
| --- | ---: | --- |
| `RELAY_SINGLE_BANDWIDTH` | `128` | 单个 Relay 会话上限，单位 Mb/s |
| `RELAY_TOTAL_BANDWIDTH` | `1024` | HBBR 进程总带宽上限，单位 Mb/s |
| `RELAY_LIMIT_SPEED` | `32` | 会话被降速后的上限，单位 Mb/s |
| `RELAY_DOWNGRADE_START_CHECK` | `1800` | 会话进入降速判定前的秒数 |
| `RELAY_DOWNGRADE_THRESHOLD` | `0.66` | 以单会话上限为基准、触发降速资格的平均用量比例 |

这些是示例显式限值。请按节点容量调整，并从 HBBR 启动日志确认实际生效值。

开放 `21117/TCP`。需要 WebSocket 时安装证书有效的 Nginx `/ws/relay` 并开放
`443/TCP`，后端 `21119/TCP` 保持私有。

## 阶段 4：中心切换到完整栈

在与引导文件**相同** Compose project 和数据路径中使用
[`examples/center/compose.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/center/compose.yaml)：

```sh
cp /path/to/repository/examples/center/compose.yaml .
docker compose --env-file .env -f compose.bootstrap.yaml down
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d
```

不要让引导 HBBS 与完整栈 HBBS 作为两个 project 长期同时运行。

Starry 的完整参考有意只包含 HBBS/HBBR 数据平面。需要账户、策略或版本化 Control API
能力时，请从不可变发布标签或镜像摘要单独部署 `rustdesk-api-kessoku` v2.8.0，并按两个
项目的文档配置内部 mTLS 与签名 Control Agent 信任边界。不要向该 Compose project 加入
未经审核的第三方 API 镜像，也不要把 Starry 私钥挂载到 API 容器。

## 阶段 5：部署 Nginx

- 中心 WSS：[`center.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/center.example.conf)
- API：[`api.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/api.example.conf)
- 每台 Relay：[`relay.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/relay.example.conf)

启用 WebSocket Signal 前阅读
[反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)。

## 验证顺序

1. 每台 Relay HBBR 正在运行且公网端口可达；
2. 等待官方健康刷新后，中心 `relay-servers` 列出预期分配池；
3. 代表性 IP 组合的 `test-geo` 返回预期第一台在线 Relay；
4. 停止一台高优先级 Relay，证明有序故障切换，再恢复；
5. 启用 WSS 时，`websocket-status` 显示正确的各 Relay native/WSS 状态；
6. 完成 native、WSS、mixed 真实会话，并在证据中对齐同一 Relay UUID；
7. 单独测试 API 登录，再在登录状态重复 Secure TCP 和远程控制。

HBBS 到 HBBR 可达不等于客户端到 Relay 的延迟或丢包。不要把中心 ping 宣称为客户端线路质量。
