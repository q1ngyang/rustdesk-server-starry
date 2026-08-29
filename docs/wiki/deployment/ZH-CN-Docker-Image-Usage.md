# Docker 镜像使用

[English](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Docker-Image-Usage) | **简体中文**

权威容器手册随源码和 Release 产物发布为
[`CONTAINER.zh-CN.md`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/container/CONTAINER.zh-CN.md)。
本页提供面向包页面的索引。

## 镜像标识

```text
ghcr.io/q1ngyang/rustdesk-server-starry
```

支持平台：

- `linux/amd64`

ARM 目前仅作为尽力的源码兼容目标；patch-v1.2.2 不承诺或发布 `linux/arm64` 镜像。

镜像包含修改后的 `hbbs`、未经修改的上游 `hbbr` 和 `rustdesk-utils`。Starry 功能只
存在于 HBBS。项目部署示例让两个服务使用同一个固定版本的 Starry 镜像，避免 HBBS 与
HBBR 因分别更新而出现版本不一致。

镜像不内置 API 服务或 MMDB 数据。

## 标签与镜像摘要

```sh
docker pull ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.2

docker buildx imagetools inspect \
  ghcr.io/q1ngyang/rustdesk-server-starry:1.1.16-patch-v1.2.2
```

使用版本标签实施可控升级；使用镜像摘要完全锁定产物。`latest` 会随新版本移动。

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

- [容器手册](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/docs/container/CONTAINER.zh-CN.md)
- [零基础单机部署教程](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Getting-Started)
- [单机 Compose](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/compose.yaml)
- [Compose ENV](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/.env.example)
- [Docker 部署](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Docker-Deployment)
- [账户与 API 服务接入](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-API-Integration)
- [版本升级与回滚](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Upgrade-and-Rollback)

GitHub 官方文档说明，已关联 package 的页面会显示
[README 等仓库信息](https://docs.github.com/zh/packages/learn-github-packages/connecting-a-repository-to-a-package)。
由于 GHCR 支持的包元数据不能设置独立完整的 README，容器专用手册以独立版本化
文档提供，并通过仓库、Release 和 OCI documentation 链接公开。
