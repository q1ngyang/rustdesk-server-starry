#!/bin/sh
set -eu

relay_data_dir=${RELAY_DATA_DIR:?RELAY_DATA_DIR must name the persistent Relay root}
case "$relay_data_dir" in
    /*) ;;
    *) echo "RELAY_DATA_DIR must be absolute" >&2; exit 78 ;;
esac
if [ ! -d "$relay_data_dir" ] || [ -L "$relay_data_dir" ]; then
    echo "RELAY_DATA_DIR must be an existing real directory" >&2
    exit 78
fi

canonical=$(readlink -f "$relay_data_dir")
if ! awk -v target="$canonical" '
    function beneath(path, mountpoint) {
        return mountpoint == "/" || path == mountpoint || index(path, mountpoint "/") == 1
    }
    {
        separator = 0
        for (i = 1; i <= NF; i++) if ($i == "-") { separator = i; break }
        mountpoint = $5
        if (separator && beneath(target, mountpoint) && length(mountpoint) > best) {
            best = length(mountpoint)
            filesystem = $(separator + 1)
        }
    }
    END { exit !(best && filesystem != "overlay" && filesystem != "tmpfs") }
' /proc/self/mountinfo; then
    echo "RELAY_DATA_DIR is not backed by an explicit persistent mount" >&2
    exit 78
fi

enrollment_dir="$relay_data_dir/starry/enrollment"
compatibility="$enrollment_dir/relay-compat.env"
if [ -f "$compatibility" ]; then
    [ ! -L "$compatibility" ] || { echo "unsafe Relay compatibility file" >&2; exit 78; }
    while IFS= read -r line || [ -n "$line" ]; do
        [ -n "$line" ] || continue
        case "$line" in
            *=*)
                # Split only at the first delimiter. A standard Base64 public
                # KEY may end in '=' padding, which is part of the exact key.
                name=${line%%=*}
                value=${line#*=}
                ;;
            *) echo "malformed Relay compatibility setting" >&2; exit 78 ;;
        esac
        [ -n "$name" ] || { echo "malformed Relay compatibility setting" >&2; exit 78; }
        case "$name" in
            KEY|STARRY_RELAY_TELEMETRY_SECRET_FILE|STARRY_RELAY_PUBLIC_ENDPOINT|STARRY_RELAY_MAX_SESSIONS|STARRY_RELAY_CAPACITY_BANDWIDTH_BPS|STARRY_RELAY_DRAINING|STARRY_RELAY_ENROLLMENT_DIR|STARRY_RELAY_FAST_MEDIA_UDP_PORT)
                case "$value" in *[!A-Za-z0-9_./:+\[\]=-]*) echo "unsafe Relay compatibility value" >&2; exit 78;; esac
                export "$name=$value"
                ;;
            *) echo "unknown Relay compatibility setting: $name" >&2; exit 78 ;;
        esac
    done < "$compatibility"
    [ -s /etc/machine-id ] || { echo "host machine identity is unavailable" >&2; exit 78; }
    [ "${STARRY_RELAY_ENROLLMENT_DIR:-}" = "$enrollment_dir" ] || {
        echo "Relay enrollment directory does not match RELAY_DATA_DIR" >&2
        exit 78
    }
else
    case "${STARRY_REQUIRE_RELAY_ENROLLMENT:-0}" in
        0|false|'') ;;
        1|true)
            echo "Relay enrollment is required but RELAY_DATA_DIR contains no enrollment" >&2
            exit 78
            ;;
        *) echo "STARRY_REQUIRE_RELAY_ENROLLMENT must be 0 or 1" >&2; exit 78 ;;
    esac
    if [ -n "${RUSTDESK_PUBLIC_KEY:-}" ]; then
        KEY=$RUSTDESK_PUBLIC_KEY
        export KEY
    else
        permits_empty_key=false
        for argument in "$@"; do
            [ "$argument" = "-k" ] && permits_empty_key=true
        done
        [ "$permits_empty_key" = true ] || {
            echo "RUSTDESK_PUBLIC_KEY is required before enrollment" >&2
            exit 78
        }
    fi
fi

cd "$relay_data_dir"
exec /usr/bin/hbbr "$@"
