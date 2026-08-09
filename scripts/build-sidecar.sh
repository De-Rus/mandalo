#!/usr/bin/env bash
# Build the mandalo CLI and stage it where Tauri expects a sidecar.
#
#   scripts/build-sidecar.sh <rust-target> [cargo profile]
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
# universal-apple-darwin is not a real compilation target. Tauri compiles each
# architecture in turn and resolves the sidecar under THAT arch's triple, so the
# universal build needs mandalo-cli-aarch64-apple-darwin and
# mandalo-cli-x86_64-apple-darwin on disk; the lipo'd
# mandalo-cli-universal-apple-darwin goes next to them for the bundling step.
# Staging only the lipo'd one fails with
#   resource path `binaries/mandalo-cli-aarch64-apple-darwin` doesn't exist
#
# The profile defaults to release-cli, what releases ship. CI passes `dev` only
# to satisfy the src-tauri build script — `cargo test --workspace` compiles
# src-tauri, which refuses to build while the declared sidecar is missing — and
# the debug artifacts it produces are the ones the test run reuses anyway.
set -euo pipefail

target=${1:?usage: build-sidecar.sh <rust-target> [cargo profile]}
profile=${2:-release-cli}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# cargo writes the `dev` profile to target/<triple>/debug, not target/<triple>/dev.
case "$profile" in
    dev) profile_dir="debug" ;;
    *) profile_dir="$profile" ;;
esac

out_dir="src-tauri/binaries"
mkdir -p "$out_dir"

case "$target" in
    *windows*) suffix=".exe" ;;
    *) suffix="" ;;
esac

build() {
    cargo build --profile "$profile" -p mandalo-cli --target "$1"
    echo "target/$1/$profile_dir/mandalo$suffix"
}

dest="$out_dir/mandalo-cli-${target}${suffix}"

if [ "$target" = "universal-apple-darwin" ]; then
    arm=$(build aarch64-apple-darwin)
    intel=$(build x86_64-apple-darwin)
    cp "$arm" "$out_dir/mandalo-cli-aarch64-apple-darwin"
    cp "$intel" "$out_dir/mandalo-cli-x86_64-apple-darwin"
    chmod 0755 "$out_dir/mandalo-cli-aarch64-apple-darwin" "$out_dir/mandalo-cli-x86_64-apple-darwin"
    lipo -create -output "$dest" "$arm" "$intel"
    lipo -info "$dest"
    ls -l "$out_dir"
else
    src=$(build "$target")
    cp "$src" "$dest"
fi

[ -n "$suffix" ] || chmod 0755 "$dest"
ls -l "$dest"
