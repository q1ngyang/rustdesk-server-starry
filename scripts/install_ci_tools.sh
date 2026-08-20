#!/bin/sh

set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    echo "usage: $0 DESTINATION" >&2
    exit 64
fi

destination=$1
actionlint_version=1.7.12
actionlint_archive=actionlint_1.7.12_linux_amd64.tar.gz
actionlint_sha256=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8
gitleaks_version=8.25.1
gitleaks_archive=gitleaks_8.25.1_linux_x64.tar.gz
gitleaks_sha256=3000d057342489827ee127310771873000b658f2987be7bbd21968ab7443913a
syft_version=1.50.0
syft_archive=syft_1.50.0_linux_amd64.tar.gz
syft_sha256=bf7b29ff57f06da30918266a0e1c2885a8f99784798d1bdb1628886aa015d788

case "$(uname -s):$(uname -m)" in
    Linux:x86_64) ;;
    *)
        echo "CI security tools are pinned only for Linux x86_64" >&2
        exit 65
        ;;
esac

mkdir -p "$destination"
tool_stage=$(mktemp -d "${TMPDIR:-/tmp}/starry-ci-tools.XXXXXX")
trap 'rm -rf "$tool_stage"' EXIT HUP INT TERM

curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
    "https://github.com/rhysd/actionlint/releases/download/v${actionlint_version}/${actionlint_archive}" \
    --output "$tool_stage/$actionlint_archive"
printf '%s  %s\n' "$actionlint_sha256" "$tool_stage/$actionlint_archive" | sha256sum --check --status
tar -xzf "$tool_stage/$actionlint_archive" -C "$tool_stage" actionlint
install -m 0755 "$tool_stage/actionlint" "$destination/actionlint"

curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
    "https://github.com/gitleaks/gitleaks/releases/download/v${gitleaks_version}/${gitleaks_archive}" \
    --output "$tool_stage/$gitleaks_archive"
printf '%s  %s\n' "$gitleaks_sha256" "$tool_stage/$gitleaks_archive" | sha256sum --check --status
tar -xzf "$tool_stage/$gitleaks_archive" -C "$tool_stage" gitleaks
install -m 0755 "$tool_stage/gitleaks" "$destination/gitleaks"

curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
    "https://github.com/anchore/syft/releases/download/v${syft_version}/${syft_archive}" \
    --output "$tool_stage/$syft_archive"
printf '%s  %s\n' "$syft_sha256" "$tool_stage/$syft_archive" | sha256sum --check --status
tar -xzf "$tool_stage/$syft_archive" -C "$tool_stage" syft
install -m 0755 "$tool_stage/syft" "$destination/syft"

"$destination/actionlint" -version
"$destination/gitleaks" version
"$destination/syft" version
