# 项目元数据与文档发布

[English](PROJECT-METADATA.md) | **简体中文**

本文记录维护中的 GitHub 元数据与文档发布流程。修改本文不代表获得自动发布授权。

## 仓库 About

默认英文描述：

> Unofficial RustDesk Server HBBS overlay with Geo Relay policy, managed MMDB,
> Secure TCP, WebSocket signalling, connection authentication, and bundled
> version-locked HBBR. API not included.

网站：

```text
https://github.com/q1ngyang/rustdesk-server-starry/wiki
```

Topics：

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

仓库描述会在有限长度内明确 HBBS、非官方身份，并说明 HBBR 已附带且与版本绑定、账户
API 不包含在项目内。

## GHCR Package 元数据

`linux/amd64` 镜像描述（工作流还会附加 patch 版本）：

> Starry HBBS for RustDesk Server with Geo Relay policy, Secure TCP, WebSocket
> signalling, connection authentication, and optional Control Agent; the same
> image bundles upstream-data-path HBBR with version reporting, while account/API services and MMDB data are
> not included

构建工作流会在镜像写入 OCI 标题、源码、文档、版本、revision、许可证和描述 label，
并在镜像写入对应元数据。文档链接指向本次构建提交中的
`docs/container/CONTAINER.md`。这样即使同一版本在文档迁移后重新构建，也不会引用
旧标签下不存在的新路径；已有镜像元数据与历史版本标签不会被重写。

GitHub 官方文档说明，已关联 package 的落地页会显示
[README 等仓库信息](https://docs.github.com/zh/packages/learn-github-packages/connecting-a-repository-to-a-package)。
Container Registry 支持的包页面 annotation 提供源码、描述和许可证，但不提供可独立
维护的完整 README。因此项目以 [`CONTAINER.zh-CN.md`](../container/CONTAINER.zh-CN.md) 提供
独立、带版本的镜像手册，把它加入 Release 产物，通过 OCI 元数据链接，同时让根
README 专注项目介绍。

## 文档导出与发布

[分类索引](../README.zh-CN.md)集中收录所有文档；只有
[`docs/wiki/`](../wiki)中的内容会导出到独立的 GitHub Wiki 仓库。
项目元数据、容器指南、版本说明、接口参考、示例说明和历史归档保留在主仓库中，
不会作为额外的 Wiki 页面发布。

[`docs/wiki/_Sidebar.md`](../wiki/_Sidebar.md)提供全局中英文索引。英文是默认主页，
每篇 Wiki 正文都有 `ZH-CN-` 对应页面。分类目录仅用于整理源文件；导出时仍按原文件名
平铺，因此现有 Wiki 地址保持不变。不同分类中出现重名文件时，导出会报错。

在仓库根目录生成本地预览，输出目录必须是新目录：

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts -p 'test_docs.py'
wiki_preview="$(mktemp -d)"
python3 scripts/export_docs.py wiki --output "${wiki_preview}/pages"
```

导出不会访问 GitHub、修改 Git 状态或覆盖已有 Wiki 输出目录。先审阅输出，再取得发布
确认；批准后只将导出的文件复制到新检出的 `rustdesk-server-starry.wiki.git` 仓库，
核对差异后再提交、推送。不要把整个 `docs` 目录或 Wiki 源文档的分类目录直接复制到
Wiki 仓库，应使用导出后的平铺文件。
如需删除某个页面，应单独审核删除操作；导出脚本不会删除 Wiki 仓库中的页面。

## Release 与镜像文档

Release 工作流已准备：

- 校验全部 Compose 文件和双语/本地文档链接；
- 导出中英文项目 README、容器指南、更新日志和当前 patch 发布说明，保留原下载文件名；
  正文中的相对源码链接转换为构建提交链接，避免附件平铺后路径失效；
- 附加完整 `examples` 与 `config` 归档，并包含迁移至 `docs/examples` 的操作说明；
- 为所有下载产物生成校验和；
- 从导出后的版本发布说明生成 GitHub Release 正文，避免维护重复的硬编码摘要。

只预览八份独立 Release 文档，不构建或发布镜像：

```sh
release_preview="$(mktemp -d)"
python3 scripts/export_docs.py release \
  --output "${release_preview}/documents" --ref "$(git rev-parse HEAD)"
```

审核尚未提交的改动时，新路径仅存在于本地；只有对应版本提交并推送后，预览中的提交
链接才可在线访问。CI 使用实际构建的 `GITHUB_SHA` 和 `GITHUB_REPOSITORY`。

## 发布门禁

最终 diff 审核通过后，发布仍是单独的明确操作：

1. 提交并推送已批准仓库变更；
2. 发布暂存的 Wiki 页面；
3. 更新 Repository About 描述、网站和 Topics；
4. 以发布模式运行 Release 工作流，或让另行批准的 Release 策略执行；
5. 核对线上 README、Wiki、Release 产物、OCI index 描述和镜像文档链接。

以上操作不得从未审核的 working tree 执行。
