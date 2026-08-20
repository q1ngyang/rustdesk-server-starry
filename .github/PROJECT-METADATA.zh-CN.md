# 项目元数据发布草案

[English](PROJECT-METADATA.md) | **简体中文**

本文记录随文档修订一同更新的 GitHub 外部元数据。它是待审草案，不代表自动发布
授权。

## 仓库 About

当前描述：

> Lightweight overlay for official RustDesk Server: ordered GEO Relay routing,
> MMDB updates, and native HBBS Secure TCP compatibility.

拟采用的默认英文描述：

> Unofficial HBBS overlay for RustDesk Server with ordered Geo Relay routing,
> managed MMDB, Secure TCP, and optional WebSocket signalling. Use official
> HBBR; API not included.

拟设置的网站：

```text
https://github.com/q1ngyang/rustdesk-server-starry/wiki
```

拟设置的 Topics：

```text
rustdesk
rustdesk-server
hbbs
self-hosted
docker
geoip
websocket
remote-desktop
rust
```

仓库描述会在有限长度内明确 HBBS、非官方身份和 HBBR/API 边界。

## GHCR Package 元数据

拟设置的 `linux/amd64` 镜像描述：

> Starry HBBS overlay for official RustDesk Server with ordered Geo Relay
> routing, Secure TCP, and optional WebSocket Signal; bundled HBBR is unmodified
> and no API is included

构建工作流会在镜像写入 OCI 标题、源码、文档、版本、revision、许可证和描述 label，
并在镜像写入对应元数据。文档 URL 直接指向 `CONTAINER.md`。

GitHub 官方文档说明，已关联 package 的落地页会显示
[README 等仓库信息](https://docs.github.com/zh/packages/learn-github-packages/connecting-a-repository-to-a-package)。
Container Registry 支持的包页面 annotation 提供源码、描述和许可证，但不提供可独立
维护的完整 README。因此项目以 [`CONTAINER.zh-CN.md`](../CONTAINER.zh-CN.md) 提供
独立、带版本的镜像手册，把它加入 Release 产物，通过 OCI 元数据链接，同时让根
README 专注项目介绍。

## Wiki 源文件与发布

待审核 Wiki 源文件位于 [`docs/wiki/`](../docs/wiki)。GitHub Wiki 是独立 Git 仓库，
只有在批准后把这些文件复制并推送到 `rustdesk-server-starry.wiki.git`，线上 Wiki 才会
改变。

`_Sidebar.md` 提供全局中英文索引。英文是默认主页，每篇叙述文档都有 `ZH-CN-`
对应页面。

## Release 与镜像文档

Release 工作流已准备：

- 校验全部 Compose 文件和双语/本地文档链接；
- 附加中英文 README、镜像手册、Changelog 和 patch Release Notes；
- 附加包含完整 `examples` 与 `config` 的归档；
- 为所有下载产物生成校验和；
- 从版本化 patch notes 生成 GitHub Release 正文，避免维护重复的硬编码摘要。

## 发布门禁

最终 diff 审核通过后，发布仍是单独的明确操作：

1. 提交并推送已批准仓库变更；
2. 发布暂存的 Wiki 页面；
3. 更新 Repository About 描述、网站和 Topics；
4. 以发布模式运行 Release 工作流，或让另行批准的 Release 策略执行；
5. 核对线上 README、Wiki、Release 产物、OCI index 描述和镜像文档链接。

以上操作不得从未审核的 working tree 执行。
