# Control Agent Compose 示例

[English](README.md) | **简体中文**

这是可选 Linux sidecar 部署。启动前先阅读完整
[Control Agent 指南](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Control-Agent)。
仓库中的配置为只读（`write_enabled: false`），mTLS API 绑定 host loopback。

在 `.env` 中把 `STARRY_PERSIST_ROOT` 设置为一个宿主机目录。默认值是 `./persist`；生产环境
可以使用绝对路径 `/www/wwwroot/rustdesk/starry`。Compose 仍分别挂载各个子目录，因此统一
宿主机根目录不会合并容器内的权限域：

```text
STARRY_PERSIST_ROOT/
├── hbbs/
├── config/
├── auth/
│   ├── secrets/
│   └── cache/
└── control/
    ├── secrets/
    ├── shared/
    └── state/
```

准备目录，不要把真实 secret 放进仓库：

```sh
cp .env.example .env
starry_persist_root=/www/wwwroot/rustdesk/starry
starry_control_uid=65532
starry_control_gid=65532

sudo install -d -m 0750 -o root -g "${starry_control_gid}" \
  "${starry_persist_root}"
sudo install -d -m 0700 -o root -g root \
  "${starry_persist_root}/hbbs" \
  "${starry_persist_root}/auth/secrets" \
  "${starry_persist_root}/auth/cache"
sudo install -d -m 0750 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" "${starry_persist_root}/config"
sudo install -d -m 0750 -o root -g "${starry_control_gid}" \
  "${starry_persist_root}/control/secrets"
sudo install -d -m 0700 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" \
  "${starry_persist_root}/control/shared" \
  "${starry_persist_root}/control/state"
sudo install -m 0640 -o "${starry_control_uid}" \
  -g "${starry_control_gid}" /dev/null \
  "${starry_persist_root}/config/config.yaml"

openssl rand -hex 32 | sudo tee \
  "${starry_persist_root}/control/shared/local-control.token" >/dev/null
sudo chown "${starry_control_uid}:${starry_control_gid}" \
  "${starry_persist_root}/control/shared/local-control.token"
sudo chmod 0600 \
  "${starry_persist_root}/control/shared/local-control.token"
```

按 `control-agent.yaml` 引用的精确文件名安装 server certificate/key、client CA 与 service
public JWKS，并放到 `control/secrets/`。这些文件保持 root 所有，且只允许所选 control group
读取。HBBS 访问 Kessoku 所需的 CA 与 client identity 放到 `auth/secrets/`，该目录在容器中
只读挂载。

把 `connection_auth.jwks.file` 设置为 `/var/lib/starry-auth/jwks.json`。它映射到宿主机
`auth/cache/jwks.json`，并允许 root HBBS 写入。备份与恢复时必须连同同目录自动生成的
`jwks.json.metadata.json` 一起保存。启用 `enforce` 前先放入有效 JWKS；不得把可写 cache
放进只读的 `auth/secrets/`。

启用写事务前，让 `config/`、`config/config.yaml` 与 `control/state/` 由该 numeric UID/GID
所有。
仅给不同 UID 所有的文件增加 group-write 权限，不足以完成保留 owner 的原子替换；启用写入
的 Agent 会在启动时拒绝该布局。保持 `control/state/` mode `0700`，受管配置不得允许
group/other 写入（通常为 `0640`），并确保所有 parent component 都是真实目录。Agent YAML
与 `control/secrets/` 下文件继续只读。

启动前验证：

```sh
docker compose --env-file .env -f compose.yaml config --quiet
```

先启动并验证 HBBS/HBBR 及原有 data path，只在受控接入窗口启动 Agent。sidecar 共享 HBBS
network namespace；HBBS 使用 Linux host networking 时，Agent 的 `127.0.0.1:21120` 只在
host loopback。只有使用 firewall 限制的私有管理 interface 时才修改 listener，绝不能
公开暴露。
