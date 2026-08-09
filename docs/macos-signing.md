# macOS signing and notarization

Unsigned builds on macOS open to *"Mándalo is damaged and can't be opened"*. The
release workflow fixes that by signing everything it ships with a **Developer ID
Application** certificate and notarizing it with Apple:

| Artifact | Signed by | Ticket stapled |
| --- | --- | --- |
| `Mandalo_*.dmg`, `mandalo.app` | `tauri-action` | yes |
| `mandalo-v*-*-apple-darwin.tar.gz` (CLI) | `scripts/macos-sign.sh` | no — bare Mach-O, Gatekeeper checks online |
| `mandalo-darwin-*.vsix` (bundled CLI) | `scripts/macos-sign.sh` | no, same reason |

Without the secrets below the workflow still builds, just unsigned — except on a
`v*` tag, where a missing credential fails the job instead of quietly shipping
something Gatekeeper will reject.

## Repository secrets

| Secret | What it is |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the password that `.p12` was exported with |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Dani Rus (TEAMID1234)` |
| `APPLE_API_KEY` | App Store Connect key id, e.g. `DHF22R7L7Y` |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID |
| `APPLE_API_KEY_P8` | base64 of `AuthKey_<key id>.p8` |

## Getting the certificate

The certificate is per-team, not per-machine — do this once and keep the `.p12`
in a password manager. The private key never leaves your machine or the manager.

```bash
# 1. A key and a certificate signing request.
openssl genrsa -out developer-id.key 2048
openssl req -new -key developer-id.key -out developer-id.csr \
    -subj "/emailAddress=you@example.com/CN=Your Name/C=RO"
```

Upload `developer-id.csr` at
<https://developer.apple.com/account/resources/certificates/add> → **Developer ID
Application** → *Previous Sub-CA* is not needed, take the default (G2) → download
`developerID_application.cer`.

```bash
# 2. Combine the downloaded certificate with the key into a .p12.
openssl x509 -inform DER -in developerID_application.cer -out developer-id.pem
openssl pkcs12 -export -legacy \
    -inkey developer-id.key -in developer-id.pem \
    -out developer-id.p12 -name "Developer ID Application"

# 3. The secret values.
base64 -i developer-id.p12 | pbcopy          # -> APPLE_CERTIFICATE
openssl x509 -in developer-id.pem -noout -subject   # -> APPLE_SIGNING_IDENTITY (the CN)
```

`-legacy` matters: without it OpenSSL 3 writes a `.p12` that macOS `security
import` refuses.

## Getting the API key

<https://appstoreconnect.apple.com/access/integrations/api> → **Keys** → generate
one with the **Developer** role. The page shows the **Issuer ID** once at the top
(`APPLE_API_ISSUER`), the key id in the row (`APPLE_API_KEY`), and lets you
download `AuthKey_<key id>.p8` exactly once.

```bash
base64 -i ~/Downloads/AuthKey_DHF22R7L7Y.p8 | pbcopy   # -> APPLE_API_KEY_P8
```

## Uploading them

```bash
gh secret set APPLE_CERTIFICATE          < <(base64 -i developer-id.p12)
gh secret set APPLE_CERTIFICATE_PASSWORD
gh secret set APPLE_SIGNING_IDENTITY
gh secret set APPLE_API_KEY
gh secret set APPLE_API_ISSUER
gh secret set APPLE_API_KEY_P8           < <(base64 -i AuthKey_DHF22R7L7Y.p8)
```

## Checking a release

```bash
spctl --assess --type execute --verbose=4 /Applications/mandalo.app
xcrun stapler validate /Applications/mandalo.app
```

For the CLI, `spctl --assess` answers `the code is valid but does not seem to be
an app` — it only assesses bundles, so that sentence is a pass, not a failure.
What actually proves a standalone binary is fine is that it runs with the
quarantine bit set:

```bash
codesign -dv --verbose=4 ./mandalo   # expect Authority=…, flags=0x10000(runtime), Timestamp=…
xattr -w com.apple.quarantine "0083;00000000;Safari;" ./mandalo
./mandalo --version
```

A bare Mach-O cannot carry a stapled ticket, so that check needs the machine to
be online the first time. It is the best available for a standalone binary.
