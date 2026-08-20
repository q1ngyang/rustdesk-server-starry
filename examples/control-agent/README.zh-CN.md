# Control Agent Compose 示例

[English](README.md) | **简体中文**

这是可选 Linux sidecar 部署。启动前先阅读完整
[Control Agent 指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)。
仓库中的配置为只读（`write_enabled: false`），mTLS API 绑定 host loopback。

准备路径，不要把真实 secret 放进仓库：

```sh
cp .env.example .env
mkdir -p data/hbbs data/starry data/control secrets
touch data/starry/config.yaml
chmod 0700 data/control
chmod 0640 data/starry/config.yaml
```

按 `control-agent.yaml` 引用的精确文件名安装 server certificate/key、client CA 与 service
public JWKS。让 `data/starry`、`data/control`、Agent YAML 与 secret file 可由 `.env`
选择的 numeric UID/GID 访问；private key 对其他用户不可读，任何 secret 都不能全局可写。

启用写事务前，让 `data/starry`、`data/starry/config.yaml` 与 `data/control` 由该 numeric
UID/GID 所有。
仅给不同 UID 所有的文件增加 group-write 权限，不足以完成保留 owner 的原子替换；启用写入
的 Agent 会在启动时拒绝该布局。保持 `data/control` mode `0700`，受管配置不得允许
group/other 写入（通常为 `0640`），并确保所有 parent component 都是真实目录。Agent YAML
与 `secrets/` 下文件继续只读。

启动前验证：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
```

先启动并验证 HBBS/HBBR 及原有 data path，只在受控接入窗口启动 Agent。sidecar 共享 HBBS
network namespace；HBBS 使用 Linux host networking 时，Agent 的 `127.0.0.1:21120` 只在
host loopback。只有使用 firewall 限制的私有管理 interface 时才修改 listener，绝不能
公开暴露。
