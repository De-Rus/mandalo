#!/usr/bin/env bash
# Write the release body for a tag, describing every asset in a directory.
#
#   scripts/release-notes.sh <tag> <assets-dir>
set -euo pipefail

tag=${1:?usage: release-notes.sh <tag> <assets-dir>}
dir=${2:?usage: release-notes.sh <tag> <assets-dir>}

list() {
    for p in "$dir"/*; do
        [ -f "$p" ] && basename "$p"
    done | sort
}

have() { list | grep -qE "$1"; }

# rows <regex> <description> — markdown table rows, one per matching asset.
# shellcheck disable=SC2016  # the backticks are markdown, not command substitution
rows() {
    list | { grep -E "$1" || true; } | while read -r f; do
        printf '| `%s` | %s |\n' "$f" "$2"
    done
}

cat <<EOF
## Mándalo $tag

The desktop app, the \`mandalo\` command line runner and the VS Code extension all
ship from this release.

### Which one do I want?

- **Desktop app** — the workbench. macOS: the \`.dmg\`. Windows: the \`.msi\`. Linux: the \`.AppImage\` (portable) or the \`.deb\`.
- **Command line** — for CI, or for driving collections from a terminal:
  \`\`\`sh
  curl -fsSL https://mandalo.dev/install.sh | sh
  \`\`\`
  or \`brew install De-Rus/tap/mandalo\`, or grab the archive for your platform below.
- **VS Code / Cursor** — install the \`.vsix\` for your platform with
  \`code --install-extension mandalo-<platform>-*.vsix\`. The platform builds bundle
  the CLI; the \`universal\` one does not, so gRPC there needs \`mandalo\` on \`PATH\`.

The builds are not signed with a paid certificate, so macOS Gatekeeper and Windows
SmartScreen will ask you to confirm the first launch.

### Verifying a download

\`SHA256SUMS\` covers every asset on this release.

\`\`\`sh
sha256sum --ignore-missing -c SHA256SUMS
\`\`\`

### Assets

| File | What it is |
| --- | --- |
EOF

rows '\.dmg$' 'macOS desktop installer'
rows '\.app\.tar\.gz$' 'macOS desktop app (updater bundle)'
rows '\.msi$' 'Windows desktop installer'
rows '(-setup|_x64-setup)\.exe$' 'Windows desktop installer (NSIS)'
rows '\.AppImage$' 'Linux desktop app, portable'
rows '\.deb$' 'Debian/Ubuntu desktop package'
rows '\.rpm$' 'Fedora/RHEL desktop package'
rows 'mandalo-v.*(apple-darwin|linux-gnu)\.tar\.gz$' 'Command line binary'
rows 'mandalo-v.*windows-msvc\.zip$' 'Command line binary'
rows '\.vsix$' 'VS Code extension'
rows '^SHA256SUMS$' 'Checksums for every asset above'

if have '\.sig$'; then
    # shellcheck disable=SC2016  # the backticks are markdown, not command substitution
    printf '\n`.sig` files are Tauri updater signatures, not something you need to download by hand.\n'
fi
