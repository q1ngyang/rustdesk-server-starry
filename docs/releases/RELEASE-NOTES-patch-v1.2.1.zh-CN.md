# patch-v1.2.1 版本说明

[English](RELEASE-NOTES-patch-v1.2.1.md) | **简体中文**

新增 Relay 版本上报：HBBR 在 WebSocket 握手中声明精确 Starry 版本，HBBS 通过现有
健康探测采集，并由 Control API v1 在 Relay 清单中返回。旧版或尚未探测的 Relay 节点
继续返回 `null`。
