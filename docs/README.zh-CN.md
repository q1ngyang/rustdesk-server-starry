# 文档分类索引

[English](README.md) | **简体中文**

除保留在根目录、作为 GitHub 项目入口的[英文 README](../README.md)外，维护中的
Markdown 文档统一分类放在 `docs/`，其中 `docs/wiki/` 仅存放在线 Wiki 源文档。
本页链接指向仓库内的源文件；需要阅读已发布的内容时，
请访问[在线 Wiki](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Home)。

## 第一次使用

1. 先看[项目概览](wiki/ZH-CN-Home.md)，了解功能与组件分工。
2. 按[快速开始](wiki/getting-started/ZH-CN-Getting-Started.md)完成不含账户/API 服务的
   单机 Docker 部署。
3. 完成[客户端配置](wiki/getting-started/ZH-CN-Client-Configuration.md)，再按
   [完整验证清单](wiki/operations/ZH-CN-Operations-and-Verification.md)逐项验收。

## 按用途查阅

| 分类 | 本地文档 |
| --- | --- |
| 项目介绍与入门 | [项目概览](wiki/ZH-CN-Home.md)、[快速开始](wiki/getting-started/ZH-CN-Getting-Started.md)、[客户端配置](wiki/getting-started/ZH-CN-Client-Configuration.md) |
| 部署 | [Docker 部署](wiki/deployment/ZH-CN-Docker-Deployment.md)、[原生部署](wiki/deployment/ZH-CN-Native-Deployment.md)、[多节点部署](wiki/deployment/ZH-CN-Multi-Node-Deployment.md)、[反向代理与 TLS](wiki/deployment/ZH-CN-Reverse-Proxy-and-TLS.md)、[账户与 API 服务接入](wiki/deployment/ZH-CN-API-Integration.md) |
| GHCR 镜像 | [独立容器使用指南](container/CONTAINER.zh-CN.md)、[Wiki 镜像说明](wiki/deployment/ZH-CN-Docker-Image-Usage.md) |
| 功能与配置 | [配置参数详解](wiki/configuration/ZH-CN-Configuration-Reference.md)、[地理位置规则入门](wiki/configuration/ZH-CN-GEO-Rules-Basics.md)、[进阶规则](wiki/configuration/ZH-CN-GEO-Rules-Advanced.md)、[连接认证](wiki/configuration/ZH-CN-Connection-Authentication.md)、[管理代理](wiki/configuration/ZH-CN-Control-Agent.md) |
| 运维 | [运维与完整验证](wiki/operations/ZH-CN-Operations-and-Verification.md)、[Relay 遥测安全与运维](wiki/ZH-CN-Relay-Telemetry-Operations.md)、[常见问题排查](wiki/operations/ZH-CN-Troubleshooting.md)、[升级与回滚](wiki/operations/ZH-CN-Upgrade-and-Rollback.md) |
| 版本说明 | [更新日志](releases/CHANGELOG.zh-CN.md)、[patch-v1.3.0](releases/RELEASE-NOTES-patch-v1.3.0.zh-CN.md)、[patch-v1.2.0](releases/RELEASE-NOTES-patch-v1.2.0.zh-CN.md)、[patch-v1.1.0](releases/RELEASE-NOTES-patch-v1.1.0.zh-CN.md) |
| 技术参考 | [架构与构建](wiki/reference/ZH-CN-Architecture-and-Build.md)、[Relay 质量协议 v1](reference/RELAY-QUALITY-PROTOCOL-v1.zh-CN.md)、[Relay Telemetry v1](reference/RELAY-TELEMETRY-v1.zh-CN.md)、[极速 Relay 授权协议 v1](reference/FAST-RELAY-AUTHORIZATION-v1.zh-CN.md)、[Profile Activation Lease v1](reference/PROFILE-ACTIVATION-LEASE-v1.zh-CN.md)、[连接认证约定](reference/auth/v1/profile.zh-CN.md)、[客户端兼容性](reference/auth/v1/client-compatibility.zh-CN.md) |
| 示例说明 | [管理代理 Compose 示例](examples/control-agent/README.zh-CN.md)；实际编排文件仍在根目录的 [examples](../examples) 中 |
| 项目维护 | [发布流程与项目元数据](project/PROJECT-METADATA.zh-CN.md)、[中文项目介绍](project/README.zh-CN.md) |
| 历史归档 | [WebSocket 开发方案](archive/PATCH-V1.1.0-WEBSOCKET-DEVELOPMENT-PLAN.md)（保留原始中文记录，不作为当前部署指南） |

## 目录结构

```text
docs/
├── README.md / README.zh-CN.md   # 本地分类索引
├── wiki/                       # 导出至在线 Wiki 的源文档
│   ├── Home.md / ZH-CN-Home.md / _Sidebar.md
│   ├── getting-started/        # 入门与客户端配置
│   ├── deployment/             # 部署与服务接入
│   ├── configuration/          # 功能、参数和规则
│   ├── operations/             # 验证、排错与升级
│   └── reference/              # 架构与构建
├── container/                  # 独立 GHCR 镜像指南
├── releases/                   # 更新日志和各版本发布说明
├── reference/                  # Relay 质量、极速 Relay、Profile activation、连接认证与兼容性参考
├── examples/control-agent/    # 示例操作说明，不存放编排文件
├── project/                    # 中文项目入口和发布维护说明
└── archive/                    # 历史开发记录
```

## 维护与发布约定

- 在所属分类中修改文档，不在根目录、`.github`、`contracts` 或编排文件目录中保留重复副本。
- Wiki 页面迁移分类时保持文件名不变。发布前只导出 `docs/wiki/`，按原文件名平铺到 Wiki
  仓库，因此在线页面地址不变；不同分类之间也不能使用重名文件。
- Wiki 中文页面使用 `ZH-CN-*.md`；其他维护中的指南使用 `*.zh-CN.md`。根目录 README
  的中文版本单独放在 `project/README.zh-CN.md`。历史记录保留原始语言。
- 只移动说明文档。配置、Compose/Nginx 示例、接口定义、测试样本和构建脚本仍保留原路径，
  不改变部署命令及运行时行为。
- 已被 Git 忽略的本地开发计划，即使放入 `archive/`，也仍然保持私有；文档检查和
  Wiki/Release 导出均不会将其纳入。
- 旧版本标签下的历史文档链接保持不变。后续 Release 文档附件仍使用原下载文件名，
  但正文中的源码链接及新镜像的文档元数据指向本次构建对应的提交。

审核前在仓库根目录执行：

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts -p 'test_docs.py'
python3 scripts/check_workflows.py
```

导出只生成本地文件，不会发布。推送仓库变更或更新 Wiki 前，请按
[文档导出与发布流程](project/PROJECT-METADATA.zh-CN.md#文档导出与发布)审核并取得确认。
