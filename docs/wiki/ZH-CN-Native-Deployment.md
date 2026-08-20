# 原生部署

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Native-Deployment) | **简体中文**

Docker 不可用，或明确要求系统级服务管理时使用原生部署。大多数 Linux 运维场景仍推荐
Docker Compose。

## Release 产物

| 平台 | 架构 | 产物 |
| --- | --- | --- |
| Debian/Ubuntu | `amd64` | HBBS、HBBR、工具和 Control Agent 独立 DEB。 |
| Linux | `amd64` | 静态 `hbbs`、`hbbr`、`rustdesk-utils`、Control Agent 和 tar 包。 |

ARM 与 Windows 仅作为非阻断兼容目标；patch-v1.2.0 不承诺对应发布制品。

Starry Release 中的 HBBR 是从相同锁定官方源码构建的未修改二进制；Starry 功能仍只在 HBBS。

只从仓库 Release 页面下载，并在安装前使用附件 `SHA256SUMS` 校验。

## Debian 与 Ubuntu 包

包名：

```text
rustdesk-server-starry-hbbs
rustdesk-server-starry-hbbr
rustdesk-server-starry-utils
```

使用包管理器安装下载文件，以便处理依赖：

```sh
sudo apt install \
  ./rustdesk-server-starry-hbbs_*_amd64.deb \
  ./rustdesk-server-starry-hbbr_*_amd64.deb \
  ./rustdesk-server-starry-utils_*_amd64.deb
```

变更流程要求时先检查包内容：

```sh
dpkg-deb --info ./rustdesk-server-starry-hbbs_*.deb
dpkg-deb --contents ./rustdesk-server-starry-hbbs_*.deb
```

安装路径：

| 用途 | 路径 |
| --- | --- |
| Starry 配置 | `/etc/rustdesk-server-starry/config.yaml` |
| 配置参考 | `/etc/rustdesk-server-starry/config.example.yaml` |
| HBBS/HBBR 工作数据 | `/var/lib/rustdesk-server-starry` |
| HBBS 服务 | `rustdesk-server-starry-hbbs.service` |
| HBBR 服务 | `rustdesk-server-starry-hbbr.service` |

安装脚本创建受限制的 `rustdesk-starry` 系统账户并启动服务。必须检查结果：

```sh
sudo systemctl status rustdesk-server-starry-hbbs --no-pager
sudo systemctl status rustdesk-server-starry-hbbr --no-pager
sudo journalctl -u rustdesk-server-starry-hbbs -n 100 --no-pager
sudo journalctl -u rustdesk-server-starry-hbbr -n 100 --no-pager
```

初始配置为空。编辑时保持 owner 和权限，然后重启 HBBS 完成首次加载：

```sh
sudoedit /etc/rustdesk-server-starry/config.yaml
sudo systemctl restart rustdesk-server-starry-hbbs
```

相对 MMDB 路径从 `/var/lib/rustdesk-server-starry` 解析，而不是从配置目录解析。

## 独立 Linux 二进制

创建专用账户和目录：

```sh
sudo useradd --system --home-dir /var/lib/rustdesk-server-starry \
  --shell /usr/sbin/nologin rustdesk-starry
sudo install -d -o rustdesk-starry -g rustdesk-starry -m 0750 \
  /var/lib/rustdesk-server-starry /etc/rustdesk-server-starry
sudo install -m 0755 ./hbbs-<release>-linux-amd64 /usr/local/bin/hbbs
sudo install -m 0755 ./hbbr-<release>-linux-amd64 /usr/local/bin/hbbr
sudo install -o rustdesk-starry -g rustdesk-starry -m 0640 /dev/null \
  /etc/rustdesk-server-starry/config.yaml
```

使用仓库 systemd 单元作为参考；若二进制位于 `/usr/local/bin`，应修改 `ExecStart`：

- [`HBBS unit`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/packaging/systemd/rustdesk-server-starry-hbbs.service)
- [`HBBR unit`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/packaging/systemd/rustdesk-server-starry-hbbr.service)

安装单元后：

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now rustdesk-server-starry-hbbs
sudo systemctl enable --now rustdesk-server-starry-hbbr
```

单机部署时两个服务使用同一受保护工作目录。不要长期以交互 root shell 运行二进制。

## Windows 二进制

本节仅保留源码构建兼容说明；v1.2.0 候选不包含或正式支持 Windows 发布制品。

本地自行构建的 Windows 文件可用于交互检查：

```powershell
$hbbsBinary = (Resolve-Path '.\hbbs-<release>-windows-amd64.exe').Path
$dataDirectory = 'C:\ProgramData\RustDeskServerStarry'
$configPath = Join-Path $dataDirectory 'starry\config.yaml'
New-Item -ItemType Directory -Path (Split-Path $configPath) -Force | Out-Null
if (-not (Test-Path -LiteralPath $configPath)) {
    New-Item -ItemType File -Path $configPath | Out-Null
}

Push-Location $dataDirectory
try {
    & $hbbsBinary "--starry-config=$configPath"
} finally {
    Pop-Location
}
```

工作目录很重要，因为身份密钥和运行状态会写入其中。示例先解析二进制路径，再从
持久化数据目录启动。

持久服务应使用支持控制台程序的服务包装器，例如 NSSM，并独立审查其来源和安装。
仓库提供可审计示例：

- [`Install-StarryServices.ps1`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/windows/Install-StarryServices.ps1)
- [`Remove-StarryServices.ps1`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/windows/Remove-StarryServices.ps1)

只在提升权限的 PowerShell 中运行安装脚本，并先检查二进制路径、数据目录、服务账户、
ACL 与防火墙。删除脚本只移除服务定义，有意保留所有数据。

Windows 上由操作员修改配置后请重启服务。旧文本管理协议已关闭，不要为管理公开代理
21115。

## 反向代理

安装方式不会改变后端端口和路径：

- HBBS `/ws/id` 后端：`21118/TCP`
- 官方 HBBR `/ws/relay` 后端：`21119/TCP`
- 可选社区 API：其独立配置的 HTTP 端口

参见[反向代理与 TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)，
只调整私有 upstream 地址；证书和精确路径要求不变。

## 验证与升级

原生进程状态不是完整验收。需要验证客户端注册、Secure TCP、真实桌面会话、HBBR
数据、Geo 分配和所有已启用 WebSocket 模式。参见
[运维与完整验证](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Operations-and-Verification)。

替换前备份工作目录和配置，保留旧二进制/包，并阅读
[版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)。
