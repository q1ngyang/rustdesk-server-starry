# patch-v1.2.2 release notes

**English** | [简体中文](RELEASE-NOTES-patch-v1.2.2.zh-CN.md)

Adds private peer-registry verification for Kessoku device discovery. The
Control Agent accepts an exact RustDesk ID and machine UUID through mTLS and a
dedicated service-JWT scope; HBBS returns only the instance ID and match result.

There is no configuration-schema or Relay-node migration. Upgrade the center
HBBS and its Control Agent before enabling Kessoku v3.0.6 discovery for clients
that are not signed in to the API.
