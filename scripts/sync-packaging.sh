#!/usr/bin/env bash
# Fill the package-manager manifests from a published release.
#
#   scripts/sync-packaging.sh v0.1.0
#
# Reads SHA256SUMS off the GitHub release and rewrites the version and every
# digest in packaging/homebrew/Formula/mandalo.rb and packaging/scoop/mandalo.json,
# so the tap and the bucket never carry a hand-typed hash. Copy the results into
# the De-Rus/homebrew-tap and De-Rus/scoop-bucket repositories afterwards.
set -euo pipefail

tag=${1:?usage: sync-packaging.sh <tag>}
case "$tag" in v*) ;; *) tag="v$tag" ;; esac

repo=${MANDALO_REPO:-De-Rus/mandalo}
base=${MANDALO_BASE_URL:-"https://github.com/$repo/releases/download/$tag"}
root=$(cd "$(dirname "$0")/.." && pwd)

sums=$(mktemp)
trap 'rm -f "$sums"' EXIT
curl -fsSL "$base/SHA256SUMS" -o "$sums" || {
    echo "sync-packaging: cannot read $base/SHA256SUMS — is $tag published?" >&2
    exit 1
}

python3 - "$root" "$tag" "$sums" <<'PY'
import json, re, sys

root, tag, sums_path = sys.argv[1:4]
version = tag.lstrip("v")

sums = {}
for line in open(sums_path).read().splitlines():
    parts = line.split(None, 1)
    if len(parts) == 2:
        sums[parts[1].strip().lstrip("*")] = parts[0]

def digest(asset):
    try:
        return sums[asset]
    except KeyError:
        sys.exit(f"sync-packaging: no digest for {asset} in SHA256SUMS")

formula_path = f"{root}/packaging/homebrew/Formula/mandalo.rb"
src = open(formula_path).read()
src = re.sub(r'^(  version )"[^"]*"', rf'\g<1>"{version}"', src, count=1, flags=re.M)

def fill(match):
    triple = match.group("triple")
    return f'{match.group("head")}sha256 "{digest(f"mandalo-{tag}-{triple}.tar.gz")}"'

src, n = re.subn(
    r'(?P<head>url "[^"]*mandalo-v#\{version\}-(?P<triple>[a-z0-9_]+-[a-z0-9-]+)\.tar\.gz"\n\s*)sha256 "[^"]*"',
    fill,
    src,
)
if n != 2:
    sys.exit(f"sync-packaging: expected 2 formula urls, rewrote {n}")
open(formula_path, "w").write(src)

cask_path = f"{root}/packaging/homebrew/Casks/mandalo.rb"
src = open(cask_path).read()
src = re.sub(r'^(  version )"[^"]*"', rf'\g<1>"{version}"', src, count=1, flags=re.M)
src, n = re.subn(
    r'^(  sha256 )"[^"]*"',
    lambda m: f'{m.group(1)}"{digest(f"mandalo_{version}_universal.dmg")}"',
    src,
    count=1,
    flags=re.M,
)
if n != 1:
    sys.exit("sync-packaging: could not rewrite the cask sha256")
open(cask_path, "w").write(src)

scoop_path = f"{root}/packaging/scoop/mandalo.json"
m = json.load(open(scoop_path))
asset = f"mandalo-{tag}-x86_64-pc-windows-msvc.zip"
m["version"] = version
m["architecture"]["64bit"]["url"] = f"https://github.com/De-Rus/mandalo/releases/download/{tag}/{asset}"
m["architecture"]["64bit"]["hash"] = digest(asset)
m["architecture"]["64bit"]["extract_dir"] = asset.removesuffix(".zip")
with open(scoop_path, "w") as fh:
    json.dump(m, fh, indent=2, ensure_ascii=False)
    fh.write("\n")

print(f"formula, cask and scoop manifest now describe {tag}")
PY

echo "copy packaging/homebrew/Formula/mandalo.rb into De-Rus/homebrew-tap"
echo "copy packaging/homebrew/Casks/mandalo.rb into De-Rus/homebrew-tap"
echo "copy packaging/scoop/mandalo.json into De-Rus/scoop-bucket"
