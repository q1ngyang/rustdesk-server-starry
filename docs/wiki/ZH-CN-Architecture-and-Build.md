# 架构与构建

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Architecture-and-Build) | **简体中文**

Starry 是可重复的源码 overlay，不是永久维护完整 RustDesk Server 树的 fork。它明确
固定官方服务端 revision，只修改 HBBS 相关源码路径，并继续使用未修改的上游 HBBR
协议约定。

## 组件和流量边界

```text
RustDesk 客户端
  |-- API HTTPS --------------------> 可选第三方 API
  |-- 原生 21116 或 WSS /ws/id ----> Starry HBBS
  |                                     | 选择一台 Relay
  |-- P2P、原生 21117 或 /ws/relay --> 官方 HBBR
```

| 组件 | Starry 是否修改 | 状态/职责 |
| --- | --- | --- |
| HBBS | 是 | Peer 注册、连接协调、Secure TCP 协商、持久 WSS 信令、Geo 判断和 Relay 分配。 |
| HBBR | 否 | 承载中继远控数据。Release 可为方便而包含从同一固定官方 revision 构建的版本。 |
| Control Agent | 独立 Starry binary | 面向一个本机 HBBS 的 Linux-only 最小权限管理 API；远程使用 mTLS/service JWT，本机使用有界 loopback 协议。 |
| `rustdesk-utils` | 否 | 上游工具的便利构建产物。 |
| API | 不包含 | 登录、地址簿、设备/管理数据；需要独立选择和加固。 |
| 客户端 | 不包含 | 选择原生或 WebSocket，并执行 P2P/HBBR 数据交换。 |

这种分层对运维很重要：API 成功不能证明 HBBS 握手；HBBS 注册不能证明 HBBR 可达；
HBBR 健康不能证明两台客户端的桌面会话。

## Overlay 目录结构

| 路径 | 用途 |
| --- | --- |
| `scripts/apply_overlay.py` | 校验唯一上游源码锚点，复制 Starry 模块/测试/配置模板并注入集成点。 |
| `overlay/src/starry_config.rs` | 严格 schema、默认值、跨字段校验、模板生成和原子配置状态。 |
| `overlay/src/geo_relay.rs` 与 `geo_relay/` | MMDB 读取/更新、信息提取、表达式编译和有序 Relay 选择。 |
| `overlay/src/secure_tcp.rs` | 兼容客户端的原生 Secure TCP 协商、认证密钥交换和分帧加密传输。 |
| `overlay/src/websocket_signal.rs` 与 `websocket_signal/` | `/ws/id` 接入、持久注册/session 路由、资源限制、有效客户端 IP 和 Relay 健康。 |
| `overlay/src/connection_auth.rs` | Ed25519 JWT/JWKS/introspection 验证及有界 metric/cache state。 |
| `overlay/src/relay_observer.rs` 与 `allocation_explain.rs` | 不可变 runtime snapshot 与共享纯 allocation decision core。 |
| `overlay/src/local_control.rs` | 有界 loopback `STARRYCTL/1` framing 与旧本地命令兼容。 |
| `overlay/src/control_agent.rs` 与 `control_agent/` | mTLS/RBAC Control API、本地 client、持久配置事务、audit、history、rollback 与 recovery。 |
| `overlay/tests/` | 真实进程 WebSocket/mixed、连接认证、local-control 与 Control Agent/fault 集成测试。 |
| `config/` | 完整 schema 示例和可部署功能模板。 |
| `docker/Dockerfile` | 包含 Release 二进制的运行镜像；默认启动 Starry HBBS。 |

应用脚本要求每个结构锚点恰好命中一次。官方源码变化使锚点无效时，构建必须停止。
CI 会应用 overlay 两次；第二次必须幂等，且 patched tree 通过 `git diff --check`。

## Relay 决策流水线

1. HBBS 获得两端有效公网地址；WSS 只有直连源位于可信代理 CIDR 时才接受转发请求头；
2. 按传输要求生成符合条件的集合：原生在线、WSS 健康，或 mixed 时取交集；
3. Geo reader 只提取已编译规则需要的信息；
4. 规则按文档顺序执行；对称规则还会交换 A/B；
5. 命中规则中第一个符合条件的 Relay 胜出；
6. 没有规则选出时，在符合条件的集合上恢复官方式轮询；
7. WSS/mixed 没有符合条件的 Relay 时返回空，而不是分配已知不兼容 endpoint。

原生 Relay 条件继续来自官方在线机制。WSS 条件来自正常 DNS/TCP/TLS、证书主机名/
证书链和精确 `/ws/relay` WebSocket Upgrade。

## 配置安全模型

- Serde 在每层 schema 拒绝未知字段；
- 激活前校验数值限制、唯一值、URL、CIDR、Origin 和所有 Relay 交叉引用；
- Starry 配置缺失、为空或无效时保持官方兼容行为，绝不应用半解析策略；
- 重载在文档层是原子的：完整有效文档会生效；空/无效文档会关闭 Starry，且不保留
  旧 Starry 状态；绝不会应用部分策略；
- MMDB 替换使用临时文件、结构/可读性检查和原子替换；失败保留上一份可读文件；
- 管理命令只应通过 HBBS 命名空间内回环地址 `21115` 使用；
- WSS 注册具备帧、队列、session、单 IP、超时和速率限制。

这些措施降低配置和暴露风险，但不能替代主机加固、密钥管理、监控、备份和真实客户端
测试。

## 从精确官方源码构建

GitHub Actions 工作流是规范流程。本地审计或开发可在支持的 Linux 构建主机执行：

```sh
git clone https://github.com/q1ngyang/rustdesk-server-starry.git
cd rustdesk-server-starry

git init _upstream
git -C _upstream remote add origin \
  https://github.com/rustdesk/rustdesk-server.git
git -C _upstream fetch --depth 1 origin 1.1.16
git -C _upstream checkout --detach FETCH_HEAD
git -C _upstream submodule update --init --recursive --depth 1

python3 scripts/apply_overlay.py _upstream
python3 scripts/apply_overlay.py _upstream
git -C _upstream diff --check

cargo metadata --manifest-path _upstream/Cargo.toml \
  --format-version 1 >/dev/null
cargo test --manifest-path _upstream/Cargo.toml --locked --lib -j 1
cargo check --manifest-path _upstream/Cargo.toml --locked --bins -j 1
cargo test --manifest-path _upstream/Cargo.toml --locked \
  --test websocket_signal -j 1 -- --nocapture
cargo test --manifest-path _upstream/Cargo.toml --locked \
  --test mixed_relay -j 1 -- --nocapture
cargo build --manifest-path _upstream/Cargo.toml --locked --release --bins
```

只能把 `1.1.16` 换成经过审核的官方 Release ref。官方 RustDesk Server 构建依赖和
Rust 工具链要求同样适用。Overlay 锚点失败表示需要审查上游变化，不能用宽泛的查找
替换绕过。

生成的 `hbbs` 包含 Starry 修改；`hbbr` 和 `rustdesk-utils` 是从同一 checkout 编译
的未修改上游源码。

## 自动化 Release 门禁

工作流解析官方 ref，并构造 `<upstream>-patch-v<PATCH_VERSION>`。发布前执行：

- Compose 静态校验；
- 精确浅克隆上游和递归子模块；
- 两次应用 overlay 的幂等性及依赖锁定检查；
- Rust 格式、全部库测试和服务端二进制检查；
- 真实进程 WSS 注册与跨传输信令测试；
- 通过未修改官方 HBBR 的 WebSocket/原生混合流量测试；
- Linux `amd64` 静态构建；
- 在 digest 固定的 Debian 测试镜像中完成 amd64 Debian 包安装和命令级 runtime 检查；
- `linux/amd64` 容器冒烟测试；
- 拼装精确的可下载候选包，其中包括 source/final-tree SPDX SBOM、确定性 archive、
  build inputs 和已验证 checksum。

只有另行批准的发布 job 具有写权限。它使用 GitHub/Sigstore artifact attestation 对
candidate checksum 和 SBOM 签名并附上可移植 bundle，随后才推送带 OCI provenance/SBOM
的 `linux/amd64` 镜像并创建或更新 GitHub Release。

ARM 仅尽力保持源码兼容，Windows 构建是非阻断实验检查；两者都不进入 patch-v1.2.0 候选。

候选构建成功本身不会修改 Release、attestation store 或 GHCR package。部署验收仍由
运维者负责。

## 镜像和产物模型

GHCR 镜像为方便包含 `hbbs`、`hbbr` 和 `rustdesk-utils`，默认命令是：

```text
hbbs --starry-config=/root/starry/config.yaml
```

推荐 Compose 有意只让 Starry 镜像运行 HBBS，并让官方 RustDesk Server 镜像运行
HBBR，从部署层清晰显示修改边界。Starry 镜像可以运行其中附带的未修改 `hbbr`，但
这不会使 HBBR 成为 Starry fork。

Release checksum 覆盖可下载文件。可移植 Sigstore bundle 与 GitHub artifact
attestation 将下载对象绑定到 build/SBOM assertion；镜像摘要、OCI provenance 和 OCI
SBOM 描述容器供应链。请按自己的信任策略验证。

## 版本维护检查表

更改上游或 patch 版本时：

1. 审查上游源码和协议变化；
2. 只有 Starry 功能/修复发布才更新 `PATCH_VERSION`；
3. 对精确候选源码重新运行 overlay；
4. 同步两种语言的 Release Notes、Changelog、镜像示例和升级说明；
5. 校验所有发布示例和相对链接；
6. 审核生成的 Release/GHCR 描述；
7. 最终文档 diff 通过人工审核后，取得明确发布批准。

## 法律和来源声明

这是非官方社区项目，与 RustDesk、MaxMind、任何 MMDB 提供方或 AI 服务提供方没有
隶属关系。项目不包含 MMDB 文件，部署者必须选择合法数据源并遵守其许可证。部分
代码和文档由 AI 协助生成或修订；它们不获得额外担保，并继续适用仓库许可条款。
