#!/bin/sh
# Mándalo CLI installer.
#
#   curl -fsSL https://mandalo.dev/install.sh | sh
#
# Downloads the mandalo binary for this machine from the latest GitHub release,
# verifies its SHA256 against the release's SHA256SUMS, and installs it without
# sudo. Environment overrides, all optional:
#
#   MANDALO_VERSION       tag to install, e.g. v0.1.0 (default: latest release)
#   MANDALO_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   MANDALO_BASE_URL      directory URL holding the assets and SHA256SUMS
#                         (default: the GitHub release download URL)

set -eu

REPO="De-Rus/mandalo"
BIN="mandalo"

say() { printf '%s\n' "$*"; }
err() { printf 'install.sh: %s\n' "$*" >&2; }
die() { err "$*"; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "\`$1\` is required but was not found on PATH"
}

http_get() {
    # http_get <url> <output-file>
    if [ -n "$DOWNLOADER" ] && [ "$DOWNLOADER" = curl ]; then
        curl -fsSL --proto '=https,http' --retry 3 -o "$2" "$1"
    else
        wget -q -O "$2" "$1"
    fi
}

resolve_downloader() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER=curl
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER=wget
    else
        die "neither curl nor wget is available"
    fi
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Darwin)
            case "$arch" in
                arm64|aarch64) echo "aarch64-apple-darwin" ;;
                x86_64) echo "x86_64-apple-darwin" ;;
                *) die "unsupported macOS architecture: $arch" ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
                aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
                *) die "unsupported Linux architecture: $arch" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            die "Windows is not installed by this script — download the .zip from https://github.com/$REPO/releases/latest or use \`scoop install mandalo\`"
            ;;
        *)
            die "unsupported operating system: $os"
            ;;
    esac
}

latest_tag() {
    # GitHub redirects /releases/latest to /releases/tag/<tag>; read the tag off
    # the final URL so this needs no API token and no JSON parser.
    if [ "$DOWNLOADER" = curl ]; then
        effective=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
            die "cannot reach GitHub to resolve the latest release"
    else
        effective=$(wget -q --max-redirect=10 -S --spider "https://github.com/$REPO/releases/latest" 2>&1 |
            sed -n 's/^.*Location: \(.*\)$/\1/p' | tail -n 1)
        [ -n "$effective" ] || die "cannot reach GitHub to resolve the latest release"
    fi
    tag=${effective##*/tag/}
    tag=${tag%%[!0-9A-Za-z._-]*}
    case "$tag" in
        v*) echo "$tag" ;;
        *) die "$REPO has no published release yet — set MANDALO_VERSION to install a specific tag" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | sed 's/^.*= //'
    else
        die "no SHA256 tool found (need sha256sum, shasum or openssl)"
    fi
}

choose_dir() {
    if [ -n "${MANDALO_INSTALL_DIR:-}" ]; then
        echo "$MANDALO_INSTALL_DIR"
        return
    fi
    user_bin="${HOME:-}/.local/bin"
    if [ -n "${HOME:-}" ]; then
        if mkdir -p "$user_bin" 2>/dev/null && [ -w "$user_bin" ]; then
            echo "$user_bin"
            return
        fi
    fi
    if [ -w /usr/local/bin ]; then
        echo /usr/local/bin
        return
    fi
    die "cannot write to \$HOME/.local/bin or /usr/local/bin — set MANDALO_INSTALL_DIR to a directory you own"
}

on_path() {
    case ":${PATH}:" in
        *":$1:"*) return 0 ;;
        *) return 1 ;;
    esac
}

main() {
    need uname
    need tar
    resolve_downloader

    target=$(detect_target)
    version=${MANDALO_VERSION:-$(latest_tag)}
    case "$version" in
        v*) ;;
        *) version="v$version" ;;
    esac
    base=${MANDALO_BASE_URL:-"https://github.com/$REPO/releases/download/$version"}

    asset="$BIN-$version-$target.tar.gz"
    say "Mándalo $version — $target"

    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t mandalo)
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "  downloading $asset"
    http_get "$base/$asset" "$tmp/$asset" ||
        die "download failed: $base/$asset"

    say "  downloading SHA256SUMS"
    http_get "$base/SHA256SUMS" "$tmp/SHA256SUMS" ||
        die "download failed: $base/SHA256SUMS — refusing to install an unverified binary"

    expected=$(grep -E "[ *]${asset}\$" "$tmp/SHA256SUMS" | head -n 1 | cut -d' ' -f1 || true)
    [ -n "$expected" ] ||
        die "SHA256SUMS has no entry for $asset — refusing to install an unverified binary"

    actual=$(sha256_of "$tmp/$asset")
    if [ "$actual" != "$expected" ]; then
        err "checksum mismatch for $asset"
        err "  expected $expected"
        err "  got      $actual"
        die "refusing to install"
    fi
    say "  checksum ok ($actual)"

    # The archive holds one directory, mandalo-<version>-<target>/, so it can be
    # unpacked anywhere without scattering files.
    tar -xzf "$tmp/$asset" -C "$tmp"
    unpacked="$tmp/$BIN-$version-$target/$BIN"
    [ -f "$unpacked" ] || die "the archive did not contain \`$BIN-$version-$target/$BIN\`"
    chmod 0755 "$unpacked"

    dir=$(choose_dir)
    mkdir -p "$dir"
    # Replace by rename so a running mandalo is never rewritten underneath itself.
    mv -f "$unpacked" "$dir/$BIN"

    say ""
    say "Installed $dir/$BIN"
    "$dir/$BIN" --version 2>/dev/null || true
    if ! on_path "$dir"; then
        say ""
        say "$dir is not on your PATH. Add it:"
        say "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile"
    fi
    say ""
    say "Try:  $BIN --help"
}

main "$@"
