#!/bin/bash

set -euo pipefail

repo="${SBRUN_INSTALL_REPO:-AnswerDotAI/sbrun}"
tag="${SBRUN_INSTALL_TAG:-}"
version="${SBRUN_INSTALL_VERSION:-}"
arch="${SBRUN_INSTALL_ARCH:-$(uname -m)}"
os="${SBRUN_INSTALL_OS:-$(uname -s)}"

dief() {
    echo "install.sh: $*" >&2
    exit 1
}

pick_prefix() {
    if [ -n "${PREFIX:-}" ]; then
        printf '%s\n' "$PREFIX"
        return
    fi
    if [ -d /opt/homebrew/bin ] || [ -d /opt/homebrew/etc ]; then
        printf '%s\n' "/opt/homebrew"
        return
    fi
    printf '%s\n' "/usr/local"
}

normalize_tag() {
    case "$1" in
        v*) printf '%s\n' "$1" ;;
        *) printf 'v%s\n' "$1" ;;
    esac
}

latest_asset_url() {
    local endpoint url
    endpoint="${SBRUN_INSTALL_LATEST_URL_ENDPOINT:-https://latest.fast.ai/latest/${repo}/.gz}"
    url="$(curl -fsSL "$endpoint")"
    case "$url" in
        http://*|https://*|file://*) printf '%s\n' "$url" ;;
        *) dief "could not determine latest release asset url from ${endpoint}" ;;
    esac
}

nearest_existing_parent() {
    local path="$1"
    while [ ! -e "$path" ]; do
        path="$(dirname "$path")"
    done
    printf '%s\n' "$path"
}

need_sudo_for_path() {
    local target parent
    target="$1"
    parent="$(nearest_existing_parent "$target")"
    [ -w "$parent" ] || return 0
    return 1
}

run_as_root() {
    if [ "${use_sudo}" = "1" ]; then
        sudo "$@"
    else
        "$@"
    fi
}

[ "$os" = "Darwin" ] || dief "this installer only supports macOS"
[ "$arch" = "arm64" ] || dief "this installer only supports Apple Silicon (arm64)"

asset_url=""
if [ -n "$version" ]; then
    tag="$(normalize_tag "$version")"
fi

if [ -n "$tag" ]; then
    asset="sbrun-${tag}-macos-${arch}.tar.gz"
    base_url="${SBRUN_INSTALL_BASE_URL:-https://github.com/${repo}/releases/download/${tag}}"
    base_url="${base_url%/}"
    asset_url="${base_url}/${asset}"
else
    asset_url="$(latest_asset_url)"
    base_url="${asset_url%/*}"
    asset="${asset_url##*/}"
fi

prefix="$(pick_prefix)"
bindir="${BINDIR:-${prefix}/bin}"
xdg_config_home="${XDG_CONFIG_HOME:-}"
if [ -n "$xdg_config_home" ]; then
    case "$xdg_config_home" in
        /*) ;;
        *) dief "XDG_CONFIG_HOME must be an absolute path" ;;
    esac
elif [ -n "${HOME:-}" ]; then
    xdg_config_home="${HOME}/.config"
else
    dief "HOME or XDG_CONFIG_HOME is required to install user config"
fi
configdir="${xdg_config_home}/sbrun"
configpath="${configdir}/config"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/sbrun-install.XXXXXX")"
cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

curl -fsSL "${asset_url}" -o "${tmpdir}/${asset}"
curl -fsSL "${base_url}/SHA256SUMS" -o "${tmpdir}/SHA256SUMS"

expected_sha="$(awk -v asset="$asset" '$2 == asset { print $1; exit }' "${tmpdir}/SHA256SUMS")"
[ -n "$expected_sha" ] || dief "could not find ${asset} in SHA256SUMS"
actual_sha="$(shasum -a 256 "${tmpdir}/${asset}" | awk '{print $1}')"
[ "$actual_sha" = "$expected_sha" ] || dief "checksum mismatch for ${asset}"

mkdir -p "${tmpdir}/pkg"
tar -C "${tmpdir}/pkg" -xzf "${tmpdir}/${asset}"

[ -f "${tmpdir}/pkg/sbrun" ] || dief "release archive is missing sbrun"
[ -f "${tmpdir}/pkg/sbrun.pl" ] || dief "release archive is missing sbrun.pl"
[ -f "${tmpdir}/pkg/sbrun.default.conf" ] || dief "release archive is missing sbrun.default.conf"

use_sudo=0
if need_sudo_for_path "$bindir"; then
    command -v sudo >/dev/null 2>&1 || dief "need sudo to install into ${prefix}, but sudo is not available"
    use_sudo=1
fi

run_as_root mkdir -p "$bindir"
mkdir -p "$configdir"
run_as_root install -m 0755 "${tmpdir}/pkg/sbrun" "${bindir}/sbrun"
run_as_root install -m 0755 "${tmpdir}/pkg/sbrun.pl" "${bindir}/sbrun.pl"
if [ ! -f "$configpath" ]; then
    install -m 0644 "${tmpdir}/pkg/sbrun.default.conf" "$configpath"
    config_note="installed"
else
    config_note="kept existing"
fi

printf 'Installed %s to %s\n' "$asset" "$bindir"
printf 'Config %s at %s\n' "$config_note" "$configpath"
