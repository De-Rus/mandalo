<img src="brand/mandalo-appicon.svg" width="72" align="left" alt="Mándalo">

# Mándalo

*¡Mándalo!* — send it. A fast, offline, git-native API client. Postman without the bloat, the forced account, or the cloud.

## Why

- **Offline, no account** — everything lives on your machine. No sign-in wall, no telemetry.
- **Git-native** — requests are plain-text [`.http`](docs/http-format.md) and [`.grpc`](docs/grpc-format.md) files in a workspace directory; environments and manifests are TOML. Diff them, review them, PR them.
- **Fast** — Tauri + Rust core. Instant startup, tiny footprint.
- **HTTP · GraphQL · gRPC** — one workbench. gRPC compiles your `.proto` files at runtime (no `protoc` needed).
- **Auth built in** — Bearer, Basic, API key (header or query).
- **Environments** — `{{variable}}` interpolation across URL, headers, body, and auth. Unresolved variables fail loud, never silently.

- **Runs in CI** — the same engine ships as `mandalo`, a command line runner with pretty/JSON/JUnit reporters, secret redaction and a credential scanner.

## Install

Three things ship from every `v*` tag, all on the same
[GitHub release](https://github.com/De-Rus/mandalo/releases/latest).

**Desktop app** — macOS `.dmg` (Apple silicon, Intel, or universal), Windows
`.msi`, Linux `.AppImage` or `.deb`. The builds are not signed with a paid
certificate, so Gatekeeper and SmartScreen ask you to confirm the first launch.

**Command line** —

```bash
curl -fsSL https://mandalo.dev/install.sh | sh   # macOS + Linux, installs to ~/.local/bin
brew install De-Rus/tap/mandalo                  # once the tap is published
```

The installer verifies the download against the release's `SHA256SUMS` and
refuses to install if it does not match. It never uses `sudo`; it prints where it
put the binary. Prebuilt archives for `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
and `x86_64-pc-windows-msvc` are on the release if you would rather do it by
hand. `cargo install mandalo-cli` does not work yet — see below.

**VS Code / Cursor** — download the `.vsix` for your platform from the release
and `code --install-extension mandalo-<platform>-0.1.0.vsix`. The platform builds
bundle the CLI, so gRPC works out of the box; the `universal` build does not, so
there gRPC needs `mandalo` on `PATH`. It is not on the Marketplace yet.

## Stack

Tauri 2 · Rust (reqwest, tonic, prost-reflect, protox) · React 19 + TypeScript + Vite · zustand

Three crates: `mandalo-core` (the product — model, workspace, transport, scripts, assertions, runner), `mandalo-cli` (the `mandalo` binary), `src-tauri` (a thin Tauri adapter over core).

## Develop

```bash
pnpm install
pnpm tauri dev
```

## Command line

```bash
mandalo run api --env staging --reporter junit > results.xml
mandalo send api auth.http#0 --env staging
mandalo ls
mandalo env list
mandalo scan --staged                     # git hygiene guard, exit 1 on a hit

mandalo listen wss://stream.example.com/socket --header "Authorization: Bearer $TOKEN"
mandalo listen https://api.example.com/events --max 10 --json
mandalo listen mqtt://broker.example.com --topic 'sensors/#' --qos 1 --timeout 30
```

`listen` opens a WebSocket, an SSE stream or an MQTT connection and prints every
event — connects, messages in and out, reconnects, drops, the close — until the
stream ends, `--max` messages have arrived or `--timeout` expires. The protocol
comes from the scheme unless `--kind ws|sse|mqtt` says otherwise (MQTT over
WebSocket needs it). `--send` fires a payload once connected, `--reconnect`
retries a dropped connection with backoff, and `--json` prints one JSON event
per line. It exits non-zero if the stream reported an error.

`run`, `send`, `ls` and `env list` all take `--reporter json` (`run` also takes
`junit`). With `--reporter json` stdout carries the JSON document and nothing
else, so it is safe to pipe straight into a parser — banners, progress and
errors go to stderr. `run` and `send` still exit non-zero when an assertion
fails or a request cannot be sent, and print the payload anyway: the failure is
described by `error`/`errorCode` on the request and by `passed` on each test.
Every test carries a stable `id` (`script:<n>` for one raised by a script)
because names are not unique. A request is addressed as `<file>#<index>`
counting from zero, the file path being collection-relative POSIX, e.g.
`auth.http#0`. A name works too when it is unique in the file
(`auth.http#Bearer auth`); the index is canonical. `mandalo ls` prints both.

Secrets never live in the workspace files: `mandalo` reads them from
`MANDALO_SECRET__<ENV>__<KEY>` and scrubs every resolved value from its own
output (and emits `::add-mask::` under GitHub Actions).

## Test

```bash
cargo test --workspace   # Rust: core + cli + tauri adapter
pnpm test                # frontend (vitest)
```

## Releasing

`.github/workflows/release.yml` is the whole distribution pipeline. Pushing a
`v*` tag builds, in parallel:

- the desktop bundles for macOS (arm64, x64, universal), Windows and Linux;
- the `mandalo` CLI for five targets, packaged as `mandalo-v<version>-<triple>.tar.gz`
  (`.zip` on Windows) with the `release-cli` profile;
- platform VSIXs plus the universal one.

A final job attaches everything to the one draft release, writes a `SHA256SUMS`
covering every asset, and generates the release notes. It leaves the release a
**draft** — publish it by hand, or `install.sh` and
`/releases/latest/download/` will not resolve. Publishing to the VS Code
Marketplace happens only when the `VSCE_PAT` secret exists; without it the
release is still complete.

```bash
make release-check   # lint + tests + installer checks, before tagging
make cli             # size-tuned local build (target/release-cli/mandalo)
make cli-dist        # package it the way the release does
```

The CLI is built with the `release-cli` profile in the workspace root
(`strip`/`lto`/`codegen-units=1`/`panic=abort`), which takes the binary from
19.8 MB to 11.8 MB. It is a separate profile on purpose: the Tauri bundle keeps
using the stock `release` profile. Cargo ignores `[profile.*]` written in a
workspace member, so it has to live in the root manifest.

Package-manager manifests (Homebrew tap, Scoop bucket) live under `packaging/`
with instructions; they are published by hand and no repository is created from
here.

### Publishing to crates.io

`cargo install mandalo-cli` does **not** work yet, and nothing is published.
`cargo package -p mandalo-cli` fails with:

> all dependencies must have a version requirement specified when packaging.
> dependency `mandalo-core` does not specify a version

To make it publishable:

1. Give the workspace dependency a version:
   `mandalo-core = { path = "crates/core", version = "0.1.0" }` in the root
   `[workspace.dependencies]`. (`mandalo-testkit` stays path-only — it is a
   dev-dependency, and Cargo strips those on publish.)
2. Add `repository = "https://github.com/De-Rus/mandalo"` to `crates/core` and
   `crates/cli` — Cargo warns about its absence today.
3. Put a `LICENSE` file inside each published crate directory, or set
   `license-file`; the root `LICENSE` is outside the package and is not
   included in the `.crate`.
4. Publish `mandalo-core` first, then `mandalo-cli` — crates.io resolves the
   versioned dependency, so the order matters.

`mandalo-testkit` and `mandalo-grpc-wasm` are already `publish = false` and stay
that way. The names `mandalo`, `mandalo-cli` and `mandalo-core` are all
unclaimed on crates.io as of this writing.

## License

MIT — see [LICENSE](LICENSE). The same file ships inside every CLI archive and
inside the VSIX.

## Roadmap

- GitHub login (Supabase Auth, PKCE) + commit/push collections to a shared repo — collaboration is just git
- Request collections persisted as workspace files (currently localStorage)
- gRPC streaming, server reflection
- Request history, cookie jar, OpenAPI import
- AI-assisted request generation
