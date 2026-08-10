# 反向代理与 TLS

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS) | **简体中文**

WebSocket Signal 需要两条不同的公网 WSS 路径。Nginx 终止 TLS；HBBS 和 HBBR
继续监听私有明文后端端口。

## 必需路径

| 公网路径 | 后端 | 用途 |
| --- | --- | --- |
| `wss://id.example.com/ws/id` | Starry HBBS `127.0.0.1:21118` | 持久身份注册与信令。 |
| `wss://relay-1.example.com/ws/relay` | 官方 HBBR `127.0.0.1:21119` | 该精确节点的 Relay 数据。 |
| `https://api.example.com/` | 可选 API `127.0.0.1:12345` | 独立账户/管理 API。 |

不要把 `/ws/id` 重写到 `/ws/relay`。也不要把所有 Relay 名称合并到一个无法对应 HBBS
实际分配 HBBR 的入口。

## 参考配置

- 完整中心 WSS：[`center.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/center.example.conf)
- 完整 Relay WSS：[`relay.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/relay.example.conf)
- 完整可选 API：[`api.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/api.example.conf)
- location 片段：[`examples/nginx/`](https://github.com/q1ngyang/rustdesk-server-starry/tree/main/examples/nginx)

替换所有示例名称和证书路径。复制到现有站点前先检查是否已有重复 `location`。

## 中心 `/ws/id`

核心契约：

```nginx
location = /ws/id {
    proxy_pass http://127.0.0.1:21118;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 120s;
    proxy_send_timeout 120s;
}
```

Nginx 与 HBBS 同机时，默认 `trusted_proxies` 回环 CIDR 可以接受转发客户端地址。
代理位于独立网络时只加入其真实来源 CIDR；不要为了读取 `X-Forwarded-For` 信任整个互联网。

## Relay `/ws/relay`

每个 Relay 域名都需要精确本机映射：

```nginx
location = /ws/relay {
    proxy_pass http://127.0.0.1:21119;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 120s;
    proxy_send_timeout 120s;
}
```

`relay_health.endpoints[].url` 必须使用该域名和精确路径。HTTPS 首页、ICMP ping 或
浏览器警告后手工忽略的证书都不是合格健康端点。

## 证书

每个域名：

1. 确认公网 DNS 指向预期入口；
2. 获取 SAN 覆盖精确名称的证书；
3. 配置完整证书链和权限受限私钥；
4. reload 前校验配置；
5. 使用正常域名验证测试。

不绕过验证地检查：

```sh
openssl s_client \
  -connect id.example.com:443 \
  -servername id.example.com \
  -verify_return_error </dev/null
```

不要使用 `curl -k`、`verify none`、原始 IP 替换，或客户端与 Starry HBBS 不信任的私有 CA。

## 校验 Nginx 与 Upgrade

```sh
sudo nginx -T
sudo nginx -t
sudo systemctl reload nginx
```

然后进行仅传输层 Upgrade 探测：

```sh
curl --http1.1 --include --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://id.example.com/ws/id
```

对每个 `https://relay-N.example.com/ws/relay` 重复。HTTP 101 只证明 HTTP/TLS
Upgrade 路径，不证明 RustDesk 注册、信令、Relay UUID 对齐或桌面数据。

## 防火墙边界

- 公网：`443/TCP` 与客户端需要的原生端口。
- 私有/本机：`21118/TCP`、`21119/TCP`、API 后端端口。
- 绝不公开代理 HBBS 管理命令入口。
- 客户端可能关闭 WebSocket 时保留原生 `21116`/`21117` 路径。

## 安全启用 WebSocket

1. 保持 `websocket_signal.enabled: false` 部署全部 Nginx 路径；
2. 校验 DNS、证书、精确 Upgrade 与后端可达；
3. 为每个 `relay_servers` 配置一个 endpoint；
4. 热加载 schema v2 并确认被接受；
5. 设置 `enabled: true`、再次加载并检查 `websocket-status`；
6. 使用真实客户端测试 WSS↔WSS 和两个方向的 mixed。

任一必需入口无法部署时保持 WebSocket Signal 关闭；原生运行可以独立继续。
