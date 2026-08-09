#!/usr/bin/env bash
# Build the mandalo CLI and stage it where Tauri expects a sidecar.
#
#   scripts/build-sidecar.sh <rust-target>
#
# Tauri resolves `externalBin: ["binaries/mandalo-cli"]` to
# src-tauri/binaries/mandalo-cli-<target triple><exe suffix> and drops it in the
# bundle as `mandalo-cli` next to the desktop binary. That is what lets the
# Homebrew cask link one CLI out of the installed .app instead of shipping a
# second copy; the cask renames it back to `mandalo` on the symlink.
#
# The name cannot be plain `mandalo`: Tauri rejects a sidecar that collides with
# the Cargo package name of src-tauri, which is `mandalo`.
#
# universal-apple-darwin is not a real compilation target: both macOS binaries
# are built and lipo'd, which is also what Tauri does with the app itself.
set -euo pipefail

target=${1:?usage: build-sidecar.sh <rust-target>}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

out_dir="src-tauri/binaries"
mkdir -p "$out_dir"

case "$target" in
    *windows*) suffix=".exe" ;;
    *) suffix="" ;;
esac

build() {
    cargo build --profile release-cli -p mandalo-cli --target "$1"
    echo "target/$1/release-cli/mandalo$suffix"
}

dest="$out_dir/mandalo-cli-${target}${suffix}"

if [ "$target" = "universal-apple-darwin" ]; then
    arm=$(build aarch64-apple-darwin)
    intel=$(build x86_64-apple-darwin)
    lipo -create -output "$dest" "$arm" "$intel"
    lipo -info "$dest"
else
    src=$(build "$target")
    cp "$src" "$dest"
fi

[ -n "$suffix" ] || chmod 0755 "$dest"
ls -l "$dest"
