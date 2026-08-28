# Native deployment

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Native-Deployment)

Use native deployment when Docker is unavailable or when system-level service
management is a deliberate requirement. Docker Compose remains the recommended
path for most Linux operators.

## Release artifacts

| Platform | Architectures | Artifacts |
| --- | --- | --- |
| Debian/Ubuntu | `amd64` | Separate HBBS, HBBR, utilities, and Control Agent DEB packages. |
| Linux | `amd64` | Static `hbbs`, `hbbr`, `rustdesk-utils`, and Control Agent binaries plus a tar archive. |

ARM and Windows remain non-blocking compatibility targets. patch-v1.2.1 does
not promise ARM or Windows release artifacts.

The Starry release HBBR is built from the same pinned official source as HBBS.
Its relay data path remains upstream; Starry adds only the bounded WebSocket
version response header used by Relay inventory.

Download only from the repository Release page and verify files against the
attached `SHA256SUMS` before installation.

## Debian and Ubuntu packages

Package names:

```text
rustdesk-server-starry-hbbs
rustdesk-server-starry-hbbr
rustdesk-server-starry-utils
```

Install downloaded files with the package manager so dependencies are handled:

```sh
sudo apt install \
  ./rustdesk-server-starry-hbbs_*_amd64.deb \
  ./rustdesk-server-starry-hbbr_*_amd64.deb \
  ./rustdesk-server-starry-utils_*_amd64.deb
```

Inspect package contents before installation when required by your change process:

```sh
dpkg-deb --info ./rustdesk-server-starry-hbbs_*.deb
dpkg-deb --contents ./rustdesk-server-starry-hbbs_*.deb
```

Installed paths:

| Purpose | Path |
| --- | --- |
| Starry configuration | `/etc/rustdesk-server-starry/config.yaml` |
| Generated configuration reference | `/etc/rustdesk-server-starry/config.example.yaml` |
| HBBS/HBBR working data | `/var/lib/rustdesk-server-starry` |
| HBBS service | `rustdesk-server-starry-hbbs.service` |
| HBBR service | `rustdesk-server-starry-hbbr.service` |

The post-install script creates a locked-down `rustdesk-starry` system account
and starts the services. Check the result instead of assuming package success:

```sh
sudo systemctl status rustdesk-server-starry-hbbs --no-pager
sudo systemctl status rustdesk-server-starry-hbbr --no-pager
sudo journalctl -u rustdesk-server-starry-hbbs -n 100 --no-pager
sudo journalctl -u rustdesk-server-starry-hbbr -n 100 --no-pager
```

The initial configuration is empty. Edit it, preserve owner and permissions,
and restart HBBS for the initial commissioning load:

```sh
sudoedit /etc/rustdesk-server-starry/config.yaml
sudo systemctl restart rustdesk-server-starry-hbbs
```

Relative MMDB paths resolve from `/var/lib/rustdesk-server-starry`, not from
`/etc/rustdesk-server-starry`.

## Standalone Linux binaries

Create a dedicated account and directories:

```sh
sudo useradd --system --home-dir /var/lib/rustdesk-server-starry \
  --shell /usr/sbin/nologin rustdesk-starry
sudo install -d -o rustdesk-starry -g rustdesk-starry -m 0750 \
  /var/lib/rustdesk-server-starry /etc/rustdesk-server-starry
sudo install -m 0755 ./hbbs-<release>-linux-amd64 /usr/local/bin/hbbs
sudo install -m 0755 ./hbbr-<release>-linux-amd64 /usr/local/bin/hbbr
sudo install -o rustdesk-starry -g rustdesk-starry -m 0640 /dev/null \
  /etc/rustdesk-server-starry/config.yaml
```

Use the repository systemd units as references, but update `ExecStart` if the
binaries are installed under `/usr/local/bin`:

- [`HBBS unit`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/packaging/systemd/rustdesk-server-starry-hbbs.service)
- [`HBBR unit`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/packaging/systemd/rustdesk-server-starry-hbbr.service)

After installing units:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now rustdesk-server-starry-hbbs
sudo systemctl enable --now rustdesk-server-starry-hbbr
```

Keep both services on the same protected working directory for a single-host
deployment. Do not run the binaries as an interactive root shell for long-term
operation.

## Windows binaries

This section is retained for source-build compatibility only. The v1.2.1
candidate does not include or support a Windows release artifact.

Locally built Windows files can be run interactively for initial inspection:

```powershell
$hbbsBinary = (Resolve-Path '.\hbbs-<release>-windows-amd64.exe').Path
$dataDirectory = 'C:\ProgramData\RustDeskServerStarry'
$configPath = Join-Path $dataDirectory 'starry\config.yaml'
New-Item -ItemType Directory -Path (Split-Path $configPath) -Force | Out-Null
if (-not (Test-Path -LiteralPath $configPath)) {
    New-Item -ItemType File -Path $configPath | Out-Null
}

Push-Location $dataDirectory
try {
    & $hbbsBinary "--starry-config=$configPath"
} finally {
    Pop-Location
}
```

The working directory matters because identity and runtime state are written
there. The example resolves the binary first, then runs it from the persistent
data directory.

For a persistent Windows service, use a service wrapper that supports console
applications, such as NSSM, and review its source and installation separately.
The repository provides auditable examples:

- [`Install-StarryServices.ps1`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/windows/Install-StarryServices.ps1)
- [`Remove-StarryServices.ps1`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/windows/Remove-StarryServices.ps1)

Run the installer script only from an elevated PowerShell after checking the
binary paths, data path, service accounts, ACLs, and firewall. The removal
script deletes service definitions but deliberately preserves all data.

On Windows, restart the service after an operator-owned configuration change.
The legacy text management protocol is disabled; do not expose port 21115
through a remote proxy.

## Reverse proxy

The backend ports and paths do not change by installation method:

- HBBS `/ws/id` backend: `21118/TCP`
- unmodified bundled HBBR `/ws/relay` backend: `21119/TCP`
- optional community API: its independently configured HTTP port

Use [Reverse Proxy and TLS](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Reverse-Proxy-and-TLS)
and adjust only the private upstream address. Certificate and exact-path
requirements remain the same.

## Verification and upgrades

Native process status is not a complete acceptance test. Verify client
registration, Secure TCP, a real desktop session, HBBR data, Geo allocation,
and each enabled WebSocket mode. See
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification).

Before replacement, back up the working directory and configuration, retain the
old binaries/packages, and read
[Upgrade and Rollback](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Upgrade-and-Rollback).
