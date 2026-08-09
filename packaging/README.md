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
in the three manifests below. The `REPLACE_WITH_SHA256_*` placeholders they carry
in git are what it fills in; `brew style` flags them until it has run, which is
expected.

The cask points at `mandalo_<version>_universal.dmg`, so it also needs that
asset to be on the release — a tag that only built the per-architecture DMGs
would make `sync-packaging.sh` fail loudly rather than emit a broken cask.

## Homebrew — `homebrew/Formula/mandalo.rb`

A bottle-less binary formula for the CLI on Linux, x86_64 and arm64. macOS is
served by the cask below, not by this. To publish it:

The tap already exists — `De-Rus/homebrew-tap`, the one that carries `sixtysix`.
`brew` maps `De-Rus/tap` to it. To publish:

1. Run `scripts/sync-packaging.sh <tag>` so the manifests carry real digests.
2. Copy the formula into the tap at `Formula/mandalo.rb` and the cask at
   `Casks/mandalo.rb`.
3. Commit and push. Users then get:

```sh
brew install De-Rus/tap/mandalo          # Linux, CLI
brew install --cask De-Rus/tap/mandalo   # macOS, app + CLI
```

Until that push lands, `brew install --cask De-Rus/tap/mandalo` fails with
"Cask 'mandalo' is unavailable" — the manifests in this directory are the source,
the tap is what Homebrew reads.

Verify before pushing:

```sh
brew audit --strict --online --formula Formula/mandalo.rb
brew install --build-from-source --formula Formula/mandalo.rb
```

## Homebrew cask — `homebrew/Casks/mandalo.rb`

The desktop app, from the universal `.dmg`. It also links the CLI: the bundle
carries it as a Tauri sidecar (`externalBin` in `tauri.conf.json`, staged by
`scripts/build-sidecar.sh`), so `binary` points into the installed `.app` rather
than downloading a second copy. Inside the bundle it is `mandalo-cli` — Tauri
rejects a sidecar named after the `src-tauri` crate, which is `mandalo` — and the
cask's `target:` puts it on `PATH` as `mandalo`. One command gets both:

```sh
brew install --cask De-Rus/tap/mandalo
```

The formula is `depends_on :linux`, so the two can never both provide
`bin/mandalo` — macOS is the cask's alone, Linux is the formula's alone, and no
`conflicts_with` is needed. This matches the `sixtysix` tap and exists because a
formula without a bottle takes Homebrew's build-from-source path, which demands
Command Line Tools to install a binary that is already compiled.

The trade-off is deliberate: on macOS there is no brew route to the CLI without
the app. Anyone who wants only the binary — a CI runner, a server — uses
`scripts/install.sh` or the `.tar.gz` from the release.

The cask needs the app signed and notarized, or every user lands on
"mandalo.app is damaged and can't be opened" — see
[`../docs/macos-signing.md`](../docs/macos-signing.md). That is also why it needs
no `postflight` quarantine strip.

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
