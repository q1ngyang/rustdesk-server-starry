# patch-v1.2.2 版本说明

[English](RELEASE-NOTES-patch-v1.2.2.md) | **简体中文**

为 Kessoku 设备发现新增私有注册表校验。Control Agent 仅通过 mTLS 与独立的服务 JWT
权限接收 RustDesk ID 和设备 UUID，HBBS 只返回实例 ID 与是否精确匹配，不泄露设备资料。

本版本不迁移配置结构，也不要求单独升级纯 Relay 节点。让未登录 API 的客户端使用
Kessoku v3.0.6 自动发现前，请先升级中心 HBBS 与其 Control Agent。
