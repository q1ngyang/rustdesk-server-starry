# rustdesk-server-starry

[English](README.md) | **简体中文**

`rustdesk-server-starry` 是官方
[`rustdesk/rustdesk-server`](https://github.com/rustdesk/rustdesk-server)
的轻量 overlay 扩展。仓库不长期维护一份改写后的上游源码；构建时先取得指定的官方版本，再由
[`scripts/apply_overlay.py`](scripts/apply_overlay.py)
在固定锚点注入 Starry 模块。

Starry 只增加两类能力：

- 按连接双方的国家、城市、ASN 和运营商信息选择 Relay，并严格按配置顺序故障切换。
- 在原生 HBBS TCP `21116` 上提供 RustDesk 客户端兼容的 Secure TCP 握手与加密收发，解决 API 登录后仍无法完成信令交互的连接超时问题。

这不是通过补充 `session_id` 字段规避问题。API 身份认证与 HBBS 信令连接是两个独立层；Starry 补齐的是登录后客户端所需的 HBBS Secure TCP 兼容能力。

> 这是非官方社区项目，与 RustDesk、MaxMind 或 MMDB 镜像提供方没有隶属关系。镜像不内置 GeoLite2 数据库；部署者应自行选择合法、可信的数据源并遵守相应许可证。

## 兼容与回退原则

- `starry/config.yaml` 首次运行时自动创建，内容为空。
- 同目录自动创建 `config.example.yaml`，且已有文件永不覆盖。
- 配置为空、YAML 无法解析或任一字段验证失败时，整份 Starry 配置不生效，HBBS 使用官方行为和官方命令行参数。
- `secure_tcp.mode: off` 保留官方原生明文 TCP；`auto` 才启用兼容协商。
- Secure TCP 仅注入原生 TCP `21116`；WebSocket/WSS `21118` 保持官方实现。
- 客户端发送合法的首个明文 Protobuf 帧时，`auto` 会兼容回退到明文；一旦客户端发送 Key Exchange，认证失败会立即关闭连接，不会不安全地降级。
- 没有 Geo 规则命中、所需 MMDB 不可用或规则中没有在线 Relay 时，继续走官方 Relay 选择逻辑。

## Docker Compose 部署

Linux 服务器优先使用 Compose。仓库中的
[`examples/compose.yaml`](examples/compose.yaml) 与
[`examples/.env.example`](examples/.env.example)
都是带完整注释的可部署示例：

```sh
cp examples/.env.example .env
cp examples/compose.yaml compose.yaml
mkdir -p data
docker compose --env-file .env up -d
```

从 GitHub Release 下载时，ENV 示例名为 `compose.env.example`：

```sh
cp compose.env.example .env
mkdir -p data
docker compose --env-file .env -f compose.yaml up -d
```

`.env` 只控制镜像标签、持久化目录、Compose 项目名、容器名和重启策略，不会注入 HBBS/HBBR。GEO、MMDB、Secure TCP 和 Relay 优先级始终由外部 YAML 管理。

示例使用 `network_mode: host`，面向 Linux Docker 主机，开放端口与官方版本一致。首次启动后会生成：

```text
data/
└── starry/
    ├── config.yaml
    └── config.example.yaml
```

编辑 `data/starry/config.yaml` 后重启 HBBS：

```sh
docker compose restart hbbs
```

也可以通过 HBBS 管理端口热加载配置：

```sh
printf 'reload-starry-config\n' | nc -w 2 127.0.0.1 21115
```

客户端仍配置同一套 ID Server、API Server 和公钥。客户端的 Relay Server 应留空；静态客户端 Relay 地址会绕过 HBBS 的动态分配。

## 外部配置

完整、可直接复制的说明见
[`config/config.example.yaml`](config/config.example.yaml)。Relay 地址始终一行一个：

```yaml
version: 1

relay_servers:
  - jp-relay-1.example.com:21117
  - jp-relay-2.example.com:21117
  - us-relay-1.example.com:21117

secure_tcp:
  mode: auto
  handshake_timeout_ms: 18000
  idle_timeout_ms: 30000
  max_frame_bytes: 65536

mmdb:
  update_interval_hours: 168
  update_on_start: true
  force_update: false
  download_timeout_seconds: 600
  minimum_bytes: 65536
  country:
    path: mmdb/GeoLite2-Country.mmdb
    url: https://example.com/GeoLite2-Country.mmdb
  city:
    path: mmdb/GeoLite2-City.mmdb
    url: https://example.com/GeoLite2-City.mmdb
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: https://example.com/GeoLite2-ASN.mmdb

geo:
  enabled: true
  rules:
    - name: East Asia
      symmetric: true
      match:
        client_a: "CN/JP/KR/TW"
        client_b: "*"
      relays:
        - jp-relay-1.example.com:21117
        - jp-relay-2.example.com:21117
```

相对 MMDB 路径以 HBBS 工作目录为基准。在 Compose 中工作目录为 `/root`，所以上例会保存到 `./data/mmdb/`。

MMDB 下载先写临时文件，再校验最小体积、MaxMind 标记及数据库可读性，最后替换旧文件。下载或验证失败时保留上一份可用数据库。`force_update: true` 会在每个更新周期强制重新下载；`update_interval_hours: 0` 关闭周期更新。

## GEO 表达式

表达式运算符为：

- `/`：OR。
- `+`：AND，优先级高于 `/`。
- `(...)`：显式分组，可任意嵌套。
- `*`：匹配任意位置。

例如：

```yaml
match:
  client_a: "(city:A城市+isp:B运营商)/city:C城市"
  client_b: "*"
```

对应 `(A城市 AND B运营商) OR C城市`。

```yaml
match:
  client_a: "((city:A城市+isp:B运营商)/(city:C城市+isp:D运营商))+country:CN"
  client_b: "*"
```

对应 `((A城市 AND B运营商) OR (C城市 AND D运营商)) AND CN国家`。

裸写的两个英文字母会按 ISO 3166-1 国家代码处理，因此多个国家可合并为：

```yaml
client_a: "CN/JP/KR/TW"
```

支持的显式字段如下：

| 字段 | 数据库 | 匹配方式 |
| --- | --- | --- |
| `continent` | Country/City | 两位洲代码，忽略大小写 |
| `country` | Country/City | 两位国家代码，忽略大小写 |
| `subdivision` / `region` | City | 子区域代码或名称 |
| `city` | City | 数据库中的任一语言城市名 |
| `geoname` / `city_id` | City | 非零 GeoNames ID |
| `asn` | ASN | 非零 ASN，可写 `AS4134` |
| `isp` / `asn_org` | ASN | 运营商名称的忽略大小写包含匹配 |

值本身包含 `/`、`+` 或括号时，用单引号或双引号包裹：

```yaml
client_a: "city:\"A/B\"+isp:'Carrier X+Y'"
```

每条规则分别匹配 `client_a` 和 `client_b`。`symmetric: true` 是默认值，表示交换连接双方后也可以匹配；方向敏感规则应设为 `false`。

## Relay 顺序与故障切换

规则从上到下匹配，规则内的 Relay 也严格从上到下选择：

```yaml
relays:
  - relay-priority-1.example.com:21117
  - relay-priority-2.example.com:21117
  - relay-priority-3.example.com:21117
```

第一台处于 HBBS 官方在线列表时始终优先，不做轮询；只有前一台被官方健康检查判定为不可用时才使用下一台。匹配规则的所有 Relay 都离线时，Starry 继续检查后续规则，最终回退到官方选择。

这里的“故障”是 HBBS 到 Relay 的可达性检查。RustDesk OSS 协议不会把客户端到各 Relay 的实时延迟、丢包和连接失败闭环上报给 HBBS，因此 Starry 不会把中心机到 Relay 的 ping 伪装成客户端线路质量。

## 查看可分配 Relay 与测试双 IP

HBBS 在本机 `21115/TCP` 提供管理命令。Docker 与本地部署使用同一套命令和选择逻辑，只是连接命令的入口不同：Compose 推荐在 HBBS 容器内执行；Linux/DEB 和 Windows 二进制直接连接本机 HBBS。

管理命令只接受来自 HBBS 所在网络命名空间回环地址的请求。不要为这些命令另外建立公网代理或远程转发。

### 查看当前可供 HBBS 分配的 Relay

配置文件中的 `relay_servers` 是完整候选池。Compose 部署查看 `data/starry/config.yaml`，DEB 部署查看 `/etc/rustdesk-server-starry/config.yaml`，直接运行二进制时查看 `--starry-config` 指定的文件（默认是当前工作目录下的 `starry/config.yaml`）。

不带参数的 `relay-servers`（简写 `rs`）显示 HBBS 当前会参与分配的列表，每行一台：

```text
jp-relay-1.example.com:21117
jp-relay-2.example.com:21117
```

多 Relay 场景下，官方 HBBS 大约每 3 秒更新一次可达性结果，因此启动或重载后应等待几秒再查询。输出顺序不代表 Starry 优先级；实际优先级始终由命中规则中的 `relays` 顺序决定。这里显示的是 HBBS 当前分配视角，也不等同于任一客户端到 Relay 的端到端线路质量。

Compose 部署：

如果在 `.env` 中修改了 `STARRY_HBBS_CONTAINER_NAME`，请将下面的
`rustdesk-starry-hbbs` 替换为修改后的值。

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115"
```

Linux 二进制或 DEB 部署：

```sh
printf 'relay-servers\n' | nc -w 2 127.0.0.1 21115
```

### 输入两个 IP，预览将分配的 Relay

`test-geo`（简写 `tg`）接受两个字面 IP 地址，并按当前 MMDB、规则和 Relay 可达状态执行与真实信令相同的选择函数：

```text
test-geo <IP_A> <IP_B>
```

第一个 IP 对应规则的 `client_a`，第二个对应 `client_b`。当规则使用默认的 `symmetric: true` 时，交换双方后也会尝试匹配；方向敏感规则应分别测试 `A B` 和 `B A`。请使用 HBBS 实际看到的客户端公网出口 IP，而不是客户端的 `192.168.x.x`、`10.x.x.x` 等内网地址；两个客户端位于同一公网 NAT 后时，两个参数应填写相同的公网 IP。

Compose 示例（请替换成两个客户端的真实公网 IP）：

```sh
docker exec rustdesk-starry-hbbs sh -c \
  "printf 'test-geo 1.1.1.1 8.8.8.8\n' | nc -w 2 127.0.0.1 21115"
```

Linux 二进制或 DEB 示例：

```sh
printf 'test-geo 1.1.1.1 8.8.8.8\n' | nc -w 2 127.0.0.1 21115
```

Windows 本地二进制可以在 PowerShell 中先定义一个本机命令函数：

```powershell
function Invoke-StarryHbbsCommand {
    param([Parameter(Mandatory)][string]$Command)

    $client = [System.Net.Sockets.TcpClient]::new()
    $result = [System.IO.MemoryStream]::new()
    try {
        $client.Connect('127.0.0.1', 21115)
        $stream = $client.GetStream()
        $stream.ReadTimeout = 2000
        $request = [System.Text.Encoding]::UTF8.GetBytes("$Command`n")
        $stream.Write($request, 0, $request.Length)
        $buffer = [byte[]]::new(4096)
        while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $result.Write($buffer, 0, $count)
        }
        [System.Text.Encoding]::UTF8.GetString($result.ToArray())
    }
    finally {
        $result.Dispose()
        $client.Dispose()
    }
}

Invoke-StarryHbbsCommand 'relay-servers'
Invoke-StarryHbbsCommand 'test-geo 1.1.1.1 8.8.8.8'
```

正常选择结果带有双引号，例如：

```text
"jp-relay-1.example.com:21117"
```

- 返回 `""` 表示当前没有可分配的 Relay。
- 完全没有输出通常表示命令格式或 IP 无法解析。
- 命中 Starry 规则时，结果会稳定选择该规则中第一台在线 Relay；在线状态变化后才会选择下一台。
- 某条匹配规则没有在线 Relay 时会继续检查后续规则；没有规则可用后结果才来自官方回退逻辑。存在多台官方 Relay 时，重复测试可能因官方轮询而返回不同节点。

该命令只预览 HBBS 此刻的分配决定，不会创建连接、不会强制两个客户端使用 Relay，也不能证明客户端到 Relay 的真实可达性。真实会话仍可能成功建立 P2P；只有会话确实进入 Relay 流程时，HBBS 才会把这个选择发送给客户端。配置重载、MMDB 更新或 Relay 健康状态在测试后发生变化时，真实会话的结果也可能随之变化。

## Secure TCP 状态机

启用 `secure_tcp.mode: auto` 后，原生 TCP `21116` 的状态机为：

```text
HBBS 发送由服务器 Ed25519 身份密钥签名的 Curve25519 公钥
  → 客户端验证签名
  → 客户端发送 Curve25519 公钥和密封的对称密钥
  → 双方使用独立收发 nonce 序列进行 Secretbox 加密收发
```

Starry 使用官方 HBBS 的身份密钥，不引入第二套共享密钥。默认 HBBS 会生成所需密钥；若显式禁用服务器身份密钥，`auto` 无法提供 Secure TCP，会保持官方明文传输。

兼容门禁覆盖：签名公钥格式、双元素 Key Exchange、密封对称密钥认证、独立收发计数器、密文认证失败、真实 TCP 加密往返，以及合法明文首帧回退。

## 本地 Release 产物

每个 Starry Release 仅提供以下架构：

| 平台 | 架构 | 产物 |
| --- | --- | --- |
| Docker | `linux/amd64`, `linux/arm64` | 同一多架构 GHCR 镜像 |
| Linux | `amd64`, `arm64` | `hbbs`、`hbbr`、`rustdesk-utils` 独立二进制及 tar.gz |
| Debian/Ubuntu | `amd64`, `arm64` | 三个互相独立的 DEB 包 |
| Windows | `amd64` | 三个独立 `.exe` 及 zip |

DEB 包名称为：

```text
rustdesk-server-starry-hbbs
rustdesk-server-starry-hbbr
rustdesk-server-starry-utils
```

HBBS DEB 首次安装即提供空配置和示例：

```text
/etc/rustdesk-server-starry/config.yaml
/etc/rustdesk-server-starry/config.example.yaml
```

服务由 systemd 管理：

```sh
sudo systemctl status rustdesk-server-starry-hbbs
sudo systemctl status rustdesk-server-starry-hbbr
```

Windows 可直接运行发布页中的独立可执行文件：

```powershell
& .\hbbs-<release>-windows-amd64.exe --starry-config=.\starry\config.yaml
```

首次运行会在当前目录创建 `starry\config.yaml` 和 `starry\config.example.yaml`。

## 自动跟随上游发布

版本格式为：

```text
<官方版本>-patch-vX.Y.Z
```

- `X`：Starry 大版本；没有重大功能或兼容性变化时不增加。
- `Y`：日常功能更新。
- `Z`：当前 patch 版本的紧急 Bug 修复。

定时流程为：

```text
发现官方最新正式 Release
  → 获取精确上游源码和子模块
  → 验证并重复应用 overlay
  → 锁定依赖并运行全部测试
  → 构建 amd64、arm64、Windows 和独立 DEB
  → 双架构容器实际启动烟测
  → 全部成功后直接发布 GitHub Release 与 GHCR 镜像
```

功能或架构失败会立即停止并创建 GitHub Issue 通知。若日志判定为 GitHub Runner 资源不足、通信中断或超时，则在 10、30、90 分钟后最多重试三次；三次仍失败才停止并通知。

首次 Starry 发布不进入自动重试控制器，并且在 README、Release 内容和镜像预览获得确认前不会发布。首次发布完成后启用仓库变量 `STARRY_RELEASE_ENABLED=true`，后续正式上游版本按上述流程自动发布，不设置人工门禁。

## Overlay 开发

本仓库保存补丁模块、注入脚本、配置模板、打包文件和工作流。验证任意官方 checkout：

```sh
python3 scripts/apply_overlay.py /path/to/clean/rustdesk-server
python3 scripts/apply_overlay.py /path/to/clean/rustdesk-server
git -C /path/to/clean/rustdesk-server diff --check
```

脚本必须可重复执行。任何固定锚点缺失或重复都会直接失败，以便在上游结构改变时停止发布，而不是生成不完整的 hard fork。

## 许可证

上游 RustDesk Server 与本项目继续遵循 `AGPL-3.0`。发布产物基于对应标签的官方源码和本仓库 overlay 构建。
