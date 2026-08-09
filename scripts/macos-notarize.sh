#!/usr/bin/env bash
# Notarize an already-signed macOS artifact and staple the ticket where possible.
#
#   scripts/macos-notarize.sh <path>
#
# Needs APPLE_API_ISSUER, APPLE_API_KEY (the key id) and APPLE_API_KEY_PATH (the
# .p8 on disk). notarytool only accepts .zip, .dmg and .pkg, so a bare binary is
# zipped for submission; a ticket cannot be stapled onto a bare Mach-O, Gatekeeper
# checks those online instead. Bundles and installers do get stapled, which is
# what makes them work on a machine that is offline the first time it opens them.
set -euo pipefail

: "${APPLE_API_ISSUER:?APPLE_API_ISSUER is not set}"
: "${APPLE_API_KEY:?APPLE_API_KEY is not set}"
: "${APPLE_API_KEY_PATH:?APPLE_API_KEY_PATH is not set}"

target=${1:?usage: macos-notarize.sh <path>}
[ -e "$target" ] || { echo "macos-notarize: $target does not exist" >&2; exit 1; }

staple=""
case "$target" in
    *.zip)
        submission=$target
        ;;
    *.dmg | *.pkg)
        submission=$target
        staple=$target
        ;;
    *)
        submission="$(mktemp -d)/$(basename "$target").zip"
        ditto -c -k --keepParent "$target" "$submission"
        case "$target" in *.app) staple=$target ;; esac
        ;;
esac

xcrun notarytool submit "$submission" \
    --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" \
    --wait --timeout 30m

if [ -n "$staple" ]; then
    xcrun stapler staple "$staple"
    xcrun stapler validate "$staple"
else
    echo "notarized $target (no ticket stapled — Gatekeeper verifies it online)"
fi
