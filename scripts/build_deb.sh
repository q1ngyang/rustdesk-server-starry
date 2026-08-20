#!/bin/sh
set -eu
umask 022

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <hbbs|hbbr|utils|agent> <binary> <amd64|arm64> <debian-version> <output-dir>" >&2
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
        description="RustDesk HBBS with the Starry overlay"
        details="Official RustDesk Server HBBS with Starry Geo Relay selection, Secure TCP, and optional WebSocket Signal."
        ;;
    hbbr)
        package="rustdesk-server-starry-hbbr"
        installed_binary="hbbr"
        description="Unmodified RustDesk HBBR bundled by Starry"
        details="Unmodified official RustDesk Server HBBR built from the same pinned upstream revision as the Starry HBBS release."
        ;;
    utils)
        package="rustdesk-server-starry-utils"
        installed_binary="rustdesk-utils"
        description="Unmodified RustDesk Server utilities bundled by Starry"
        details="Unmodified official RustDesk Server utilities built from the same pinned upstream revision as the Starry HBBS release."
        ;;
    agent)
        package="rustdesk-server-starry-control-agent"
        installed_binary="starry-control-agent"
        description="Least-privilege Starry Control Agent"
        details="Versioned mTLS and scoped service-JWT management API for one local Starry HBBS instance, including atomic configuration transactions."
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

if [ ! -f "$binary" ] || [ -L "$binary" ]; then
    echo "binary not found: $binary" >&2
    exit 66
fi

if ! dpkg --validate-version "$version" >/dev/null 2>&1; then
    echo "invalid Debian version: $version" >&2
    exit 64
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
 $details
EOF

if [ "$component" = "hbbs" ] || [ "$component" = "hbbr" ] || [ "$component" = "agent" ]; then
    if [ "$component" = "agent" ]; then
        service="rustdesk-server-starry-control-agent"
    else
        service="rustdesk-server-starry-$component"
    fi
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
install -d -o root -g rustdesk-starry -m 0750 /var/lib/rustdesk-server-starry
if ! getent passwd rustdesk-starry >/dev/null 2>&1; then
    adduser --system --ingroup rustdesk-starry --home /var/lib/rustdesk-server-starry \
        --no-create-home --disabled-password rustdesk-starry
fi
install -d -o rustdesk-starry -g rustdesk-starry -m 0750 /var/lib/rustdesk-server-starry
EOF
    if [ "$component" = "hbbs" ] || [ "$component" = "agent" ]; then
        cat >> "$package_root/DEBIAN/postinst" <<'EOF'
install -d -o root -g rustdesk-starry -m 0770 \
    /etc/rustdesk-server-starry/managed
if [ -f /etc/rustdesk-server-starry/managed/config.yaml ]; then
    chown rustdesk-starry:rustdesk-starry /etc/rustdesk-server-starry/managed/config.yaml
    chmod 0640 /etc/rustdesk-server-starry/managed/config.yaml
fi
if [ ! -e /etc/rustdesk-server-starry/local-control.token ]; then
    umask 077
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n' > \
        /etc/rustdesk-server-starry/local-control.token
fi
chown rustdesk-starry:rustdesk-starry \
    /etc/rustdesk-server-starry/local-control.token
chmod 0600 /etc/rustdesk-server-starry/local-control.token
EOF
    fi
    if [ "$component" = "agent" ]; then
        cat >> "$package_root/DEBIAN/postinst" <<'EOF'
chown root:rustdesk-starry /etc/rustdesk-server-starry/control-agent.yaml
chmod 0640 /etc/rustdesk-server-starry/control-agent.yaml
EOF
    fi
    cat >> "$package_root/DEBIAN/postinst" <<'EOF'
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
EOF
    if [ "$component" != "agent" ]; then
        cat >> "$package_root/DEBIAN/postinst" <<EOF
    systemctl enable --now $service.service || true
EOF
    fi
    cat >> "$package_root/DEBIAN/postinst" <<'EOF'
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

if [ "$component" = "agent" ]; then
    mkdir -p "$package_root/etc/rustdesk-server-starry"
    install -m 0640 "$repo_root/config/control-agent.example.yaml" \
        "$package_root/etc/rustdesk-server-starry/control-agent.yaml"
    cat > "$package_root/DEBIAN/conffiles" <<'EOF'
/etc/rustdesk-server-starry/control-agent.yaml
EOF
fi

if [ "$component" = "hbbs" ]; then
    mkdir -p "$package_root/etc/rustdesk-server-starry/managed"
    : > "$package_root/etc/rustdesk-server-starry/managed/config.yaml"
    install -m 0644 "$repo_root/config/config.example.yaml" \
        "$package_root/etc/rustdesk-server-starry/config.example.yaml"
    cat > "$package_root/DEBIAN/conffiles" <<'EOF'
/etc/rustdesk-server-starry/managed/config.yaml
/etc/rustdesk-server-starry/config.example.yaml
EOF
fi

# Make repeated builds from identical binary/configuration inputs byte-for-byte
# reproducible.  dpkg-deb uses SOURCE_DATE_EPOCH for archive headers, while the
# explicit tree normalization covers data/control tar members.
find "$package_root" -exec touch -h -d '@0' {} +
SOURCE_DATE_EPOCH=0 dpkg-deb --build --root-owner-group -Zgzip \
    "$package_root" "$output_dir/${package}_${version}_${architecture}.deb"
