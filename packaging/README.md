# Packaging

Manifests for the package managers that distribute the `mandalo` CLI. Nothing here
runs in CI — publishing to a tap or to a manifest repository is a deliberate,
manual step, and none of these repositories are created by this repo.

Every manifest points at the archives a `v*` tag publishes on the GitHub release:

```
mandalo-v<version>-aarch64-apple-darwin.tar.gz
mandalo-v<version>-x86_64-apple-darwin.tar.gz
mandalo-v<version>-x86_64-unknown-linux-gnu.tar.gz
mandalo-v<version>-aarch64-unknown-linux-gnu.tar.gz
mandalo-v<version>-x86_64-pc-windows-msvc.zip
```

Each archive holds one directory named after itself
(`mandalo-v<version>-<triple>/mandalo`), so extracting one in a populated
directory cannot scatter files over what is already there. Homebrew strips that
single root directory by itself; the Scoop manifest names it in `extract_dir`,
and `sync-packaging.sh` keeps that in step with the version.

The digests they need are the ones in `SHA256SUMS` on the same release, and
nobody should be typing those by hand:

```sh
# after the release is published
scripts/sync-packaging.sh v0.1.0
```

That reads `SHA256SUMS` off the release and rewrites the version and every digest
in both manifests below. The `REPLACE_WITH_SHA256_*` placeholders they carry in
git are what it fills in; `brew style` flags them until it has run, which is
expected.

## Homebrew — `homebrew/Formula/mandalo.rb`

A bottle-less binary formula, four platforms. To publish it:

1. Create a public repository named **`homebrew-tap`** under `De-Rus`. The name
   matters: `brew` maps `De-Rus/tap` to `De-Rus/homebrew-tap`.
2. Run `scripts/sync-packaging.sh <tag>` so the formula carries real digests.
3. Copy it into the tap at `Formula/mandalo.rb`.
4. Commit and push. Users then get:

```sh
brew install De-Rus/tap/mandalo
```

Verify before pushing:

```sh
brew audit --strict --online --formula Formula/mandalo.rb
brew install --build-from-source --formula Formula/mandalo.rb
```

## Scoop — `scoop/mandalo.json`

A Scoop manifest for Windows. Same shape: create a `De-Rus/scoop-bucket`
repository, run `scripts/sync-packaging.sh <tag>`, drop the manifest in at
`bucket/mandalo.json`, and users get:

```sh
scoop bucket add mandalo https://github.com/De-Rus/scoop-bucket
scoop install mandalo
```

`checkver`/`autoupdate` are wired to the GitHub releases page, so
`scoop update mandalo` picks up new tags without editing the manifest by hand.

## winget — not packaged

winget is not shipped here on purpose. It is not a manifest you host: every
version has to be submitted as a pull request to
[`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs), which
requires the installer to be an `.msi`/`.exe` (a bare `.zip` of a binary is
accepted only as a portable package, a separate manifest type), and the
submission is reviewed and static-analysis-scanned. That is a per-release human
loop, not a file to commit. Worth doing once the desktop `.msi` has a stable
home; not worth pretending it is automated today.

## crates.io — not publishable yet

`cargo install mandalo-cli` cannot work until the crates carry versioned
dependencies on each other. See the "Publishing to crates.io" section in
`../README.md` for exactly what has to change.
