#!/usr/bin/env bash
# Package the built mandalo CLI for one target.
#
#   scripts/package-cli.sh <rust-target> [version]
#
# Reads target/<triple>/release-cli/mandalo and writes
# target/dist/mandalo-<version>-<triple>.tar.gz (.zip on Windows).
# Only "asset=<path>" goes to stdout so the caller can append it to $GITHUB_OUTPUT.
set -euo pipefail

target=${1:?usage: package-cli.sh <rust-target> [version]}
version=${2:-}

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

case "$version" in
    v*) ;;
    "") version="v$(node -p "require('./package.json').version")" ;;
    *) version="v$version" ;;
esac

name="mandalo-${version}-${target}"
stage="target/staging/$name"
out="target/dist"
rm -rf "$stage"
mkdir -p "$stage" "$out"
cp LICENSE README.md "$stage/"

if [ ! -d "target/$target/release-cli" ]; then
    echo "package-cli: target/$target/release-cli is missing — build it first with" >&2
    echo "  cargo build --profile release-cli -p mandalo-cli --target $target" >&2
    exit 1
fi

if [ -f "target/$target/release-cli/mandalo.exe" ]; then
    cp "target/$target/release-cli/mandalo.exe" "$stage/"
    asset="$out/${name}.zip"
    rm -f "$asset"
    (cd "$stage" && 7z a -tzip "$root/$asset" ./* >/dev/null)
else
    cp "target/$target/release-cli/mandalo" "$stage/"
    chmod 0755 "$stage/mandalo"
    asset="$out/${name}.tar.gz"
    rm -f "$asset"
    tar -czf "$asset" -C "$stage" .
fi

ls -l "$asset" >&2
echo "asset=$asset"
