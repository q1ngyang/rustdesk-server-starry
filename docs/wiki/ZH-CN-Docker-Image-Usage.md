# Docker 镜像使用

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Image-Usage) | **简体中文**

权威容器手册随源码和 Release 产物发布为
[`CONTAINER.zh-CN.md`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CONTAINER.zh-CN.md)。
本页提供面向包页面的索引。

## 镜像标识

```text
ghcr.io/q1ngyang/rustdesk-server-starry
```

支持平台：

- `linux/amd64`
- `linux/arm64`

镜像包含修改后的 `hbbs`、未修改上游 `hbbr` 和未修改 `rustdesk-utils`。
Starry 功能只存在于 HBBS。推荐部署使用官方 `rustdesk/rustdesk-server` 镜像运行
HBBR，使该边界保持清晰。

镜像不内置 API 服务或 MMDB 数据。

## 标签与 digest

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.1.0
```

使用版本标签实施可控升级；使用 digest 完全锁定产物。`latest` 是移动引用。

## 命令

```sh
docker run --rm IMAGE hbbs --help
docker run --rm IMAGE hbbr --help
docker run --rm IMAGE rustdesk-utils --help
```

将 `IMAGE` 替换为完整标签或 digest。`hbbs` 是镜像默认命令；使用项目 Compose
启动时读取 `/root/starry/config.yaml`。

## 存储契约

将一个持久目录挂载到 `/root`，其中包含：

- 服务器身份密钥；
- RustDesk 数据库；
- 日志与运行状态；
- `starry/config.yaml` 和自动生成的示例；
- 相对路径引用的 MMDB 文件。

应整体备份该目录。替换容器时不得意外生成新的服务器身份。

## 从这里开始

- [容器手册](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/CONTAINER.zh-CN.md)
- [单机 Compose](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [Compose ENV](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/.env.example)
- [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)
- [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)

GitHub 官方文档说明，已关联 package 的页面会显示
[README 等仓库信息](https://docs.github.com/zh/packages/learn-github-packages/connecting-a-repository-to-a-package)。
由于 GHCR 支持的包元数据不含独立完整 README annotation，容器专用手册以独立版本化
文档提供，并通过仓库、Release 和 OCI documentation 链接公开。
