# patch-v1.2.1 release notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.2.1.zh-CN.md)

Adds Relay version reporting: HBBR advertises its exact Starry version during
the WebSocket handshake, HBBS records it through the existing health probe,
and Control API v1 exposes it in the Relay inventory. Legacy or unprobed Relay
nodes continue to report `null`.
