#!/bin/sh
set -eu

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <hbbs|hbbr|utils> <binary> <amd64|arm64> <debian-version> <output-dir>" >&2
    exit 64
fi

component="$1"
binary="$2"
architecture="$3"
version="$4"
output_dir="$5"

case "$component" in
    hbbs)
        package="rustdesk-server-starry-hbbs"
        installed_binary="hbbs"
        description="RustDesk Starry ID and rendezvous server"
        ;;
    hbbr)
        package="rustdesk-server-starry-hbbr"
        installed_binary="hbbr"
        description="RustDesk Starry relay server"
        ;;
    utils)
        package="rustdesk-server-starry-utils"
        installed_binary="rustdesk-utils"
        description="RustDesk Starry server utilities"
        ;;
    *)
        echo "unsupported component: $component" >&2
        exit 64
        ;;
esac

case "$architecture" in
    amd64|arm64) ;;
    *)
        echo "unsupported Debian architecture: $architecture" >&2
        exit 64
        ;;
esac

if [ ! -f "$binary" ]; then
    echo "binary not found: $binary" >&2
    exit 66
fi

repo_root="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
build_root="$(mktemp -d)"
trap 'rm -rf "$build_root"' EXIT INT TERM

package_root="$build_root/$package"
mkdir -p "$package_root/DEBIAN" "$package_root/usr/bin" "$output_dir"
install -m 0755 "$binary" "$package_root/usr/bin/$installed_binary"

cat > "$package_root/DEBIAN/control" <<EOF
Package: $package
Version: $version
Section: net
Priority: optional
Architecture: $architecture
Maintainer: rustdesk-server-starry maintainers
Depends: ca-certificates, adduser
Homepage: https://github.com/q1ngyang/rustdesk-server-starry
Description: $description
 Official rustdesk-server with the small Starry GEO Relay and Secure TCP overlay.
EOF

if [ "$component" = "hbbs" ] || [ "$component" = "hbbr" ]; then
    service="rustdesk-server-starry-$component"
    mkdir -p "$package_root/lib/systemd/system"
    install -m 0644 \
        "$repo_root/packaging/systemd/$service.service" \
        "$package_root/lib/systemd/system/$service.service"

    cat > "$package_root/DEBIAN/postinst" <<EOF
#!/bin/sh
set -e
if ! getent group rustdesk-starry >/dev/null 2>&1; then
    addgroup --system rustdesk-starry
fi
if ! getent passwd rustdesk-starry >/dev/null 2>&1; then
    adduser --system --ingroup rustdesk-starry --home /var/lib/rustdesk-server-starry \
        --no-create-home --disabled-password rustdesk-starry
fi
install -d -o rustdesk-starry -g rustdesk-starry -m 0750 /var/lib/rustdesk-server-starry
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
    systemctl enable --now $service.service || true
fi
exit 0
EOF
    chmod 0755 "$package_root/DEBIAN/postinst"

    cat > "$package_root/DEBIAN/prerm" <<EOF
#!/bin/sh
set -e
if [ "\${1:-}" = remove ] && command -v systemctl >/dev/null 2>&1; then
    systemctl disable --now $service.service || true
fi
exit 0
EOF
    chmod 0755 "$package_root/DEBIAN/prerm"
fi

if [ "$component" = "hbbs" ]; then
    mkdir -p "$package_root/etc/rustdesk-server-starry"
    : > "$package_root/etc/rustdesk-server-starry/config.yaml"
    install -m 0644 "$repo_root/config/config.example.yaml" \
        "$package_root/etc/rustdesk-server-starry/config.example.yaml"
    cat > "$package_root/DEBIAN/conffiles" <<'EOF'
/etc/rustdesk-server-starry/config.yaml
/etc/rustdesk-server-starry/config.example.yaml
EOF
fi

dpkg-deb --build --root-owner-group "$package_root" \
    "$output_dir/${package}_${version}_${architecture}.deb"
