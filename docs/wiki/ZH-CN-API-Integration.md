# 账户与 API 服务接入

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/API-Integration) | **简体中文**

Starry 不包含账户与 API 服务。设备注册、HBBS 信令、HBBR 中继流量、账户登录和可选的
Starry 管理代理分别由不同服务负责。因此，登录成功不能证明信令或中继正常，Starry
部署成功也不会自动提供账户功能。

## 选择 API 服务

Starry 可以搭配兼容的第三方 RustDesk API 服务。选用前应确认其支持的 RustDesk 客户端
版本、服务器公钥使用方式、数据库备份流程、认证机制，以及连接令牌是否符合 Starry
连接认证要求。

为了获得更一致的联合使用体验，推荐同一开发者维护的
[`q1ngyang/rustdesk-api-kessoku`](https://github.com/q1ngyang/rustdesk-api-kessoku)。
Kessoku 的安装、TLS、数据库、管理员、客户端、备份和 Starry 对接要求，请以
[Kessoku Wiki](https://github.com/q1ngyang/rustdesk-api-kessoku/wiki)为准。

Kessoku + Starry 联合部署专页仍在编写中。该页面链接补充到本文之前，请先在关闭连接
认证的状态下独立部署并验证 Starry，再按照 Kessoku Wiki 部署 API 服务。不要根据
Starry 的通用 API 示例猜测 Kessoku 的内部端口、路径、令牌字段或反向代理规则。

## 各组件分别负责什么

| 服务 | 负责 | 不负责 |
| --- | --- | --- |
| Starry HBBS | ID 注册、信令、安全 TCP、按地理位置选择中继服务器、可选连接令牌校验 | 账户、地址簿、网页管理或中继数据转发 |
| Starry 镜像内的 HBBR | 原生和 WebSocket 中继数据转发 | 账户、地理位置策略或签发令牌 |
| 账户/API 服务 | 登录、账户与设备数据，以及该 API 项目文档列出的功能 | HBBS 信令或 HBBR 中继传输 |
| Starry 管理代理 | 可选的单机 HBBS 私有管理接口 | 公网账户 API 或 RustDesk 客户端登录接口 |

## 推荐接入顺序

1. 先完成[快速开始](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)，
   在不使用 API 的情况下验证原生连接和中继连接。
2. 按照 API 项目自己的说明，使用独立域名和持久化目录部署 API。
3. 分别验证 HTTPS、管理员登录、数据库备份和 RustDesk 客户端登录。
4. 在客户端填写“API 服务器”；“ID 服务器”、服务器公钥和留空的“中继服务器”保持不变。
5. 再次完成真实原生会话和强制中继会话。登录成功不能代替传输验收。
6. 如果 API 明确支持 Starry 连接令牌，配置信任材料期间保持
   `connection_auth.mode: off`，随后切换到 `audit`（仅记录），核对日志无误后再使用
   `enforce`（强制拦截）。
7. 如果需要管理代理，应先按只读模式接入，并只通过
   [管理代理文档](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)
   规定的私有 mTLS 管理通道访问。

## 反向代理和防火墙

如果 API 项目建议使用独立域名，应将 ID/WSS 入口与账户 API 分开。Starry 的明文
WebSocket 后端 `21118/TCP`、`21119/TCP` 不得对公网开放；管理代理 `21120/TCP` 只应
监听本机或私有管理网络。

仓库中的 `examples/nginx/api.example.conf` 只是通用占位示例，不是 Kessoku 的部署
规范。Kessoku 可能区分公网接口与内部接口，并有独立的信任要求；所有 API 反向代理
设置应以 Kessoku Wiki 为准。

## 回退方法

如果接入 API 后客户端无法登录，可清空客户端的“API 服务器”设置或恢复上一版 API，
不要更换 Starry 服务器身份密钥。如果连接认证阻断会话，将模式恢复为 `audit` 或
`off`，确认有效配置已加载后，再次验证原生连接和中继连接。API 数据和 Starry 数据应
分别备份。
