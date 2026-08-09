#!/usr/bin/env bash
# Sign macOS binaries with the Developer ID Application certificate.
#
#   scripts/macos-sign.sh <path> [path...]
#
# Needs APPLE_CERTIFICATE (base64 of the .p12), APPLE_CERTIFICATE_PASSWORD and
# APPLE_SIGNING_IDENTITY. The certificate is imported into a throwaway keychain
# under $RUNNER_TEMP, never the login keychain, and the keychain is reused
# across calls so a job that signs several binaries imports the cert once.
#
# The desktop bundles do NOT go through here: tauri-action reads the same three
# variables and signs the .app and the .dmg itself.
set -euo pipefail

: "${APPLE_CERTIFICATE:?APPLE_CERTIFICATE is not set}"
: "${APPLE_CERTIFICATE_PASSWORD:?APPLE_CERTIFICATE_PASSWORD is not set}"
: "${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is not set}"
[ "$#" -gt 0 ] || { echo "usage: macos-sign.sh <path> [path...]" >&2; exit 1; }

keychain="${RUNNER_TEMP:-/tmp}/mandalo-signing.keychain-db"
password_file="${RUNNER_TEMP:-/tmp}/mandalo-signing.password"

if [ ! -f "$keychain" ]; then
    umask 077
    openssl rand -hex 24 > "$password_file"
    password=$(cat "$password_file")

    security create-keychain -p "$password" "$keychain"
    # -lut 21600: no auto-lock on idle, hard timeout at 6h so a hung job cannot
    # leave an unlocked keychain behind forever.
    security set-keychain-settings -lut 21600 "$keychain"
    security unlock-keychain -p "$password" "$keychain"

    p12="${RUNNER_TEMP:-/tmp}/mandalo-signing.p12"
    printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$p12"
    security import "$p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" \
        -T /usr/bin/codesign -T /usr/bin/security
    rm -f "$p12"

    # Without this codesign prompts for the keychain password on every use, which
    # in CI means it hangs until the step times out.
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null

    # Prepending rather than replacing: dropping the runner's own keychains from
    # the search list breaks anything else in the job that needs them.
    existing=()
    while IFS= read -r line; do
        existing+=("$(printf '%s' "$line" | tr -d ' "')")
    done < <(security list-keychains -d user)
    security list-keychains -d user -s "$keychain" "${existing[@]}"
fi

security unlock-keychain -p "$(cat "$password_file")" "$keychain"

for path in "$@"; do
    [ -e "$path" ] || { echo "macos-sign: $path does not exist" >&2; exit 1; }
    # --options runtime is what notarization requires; --timestamp is what keeps
    # the signature valid after the certificate expires.
    codesign --force --options runtime --timestamp \
        --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$path"
    codesign --verify --strict --verbose=2 "$path"
    echo "signed $path"
done
