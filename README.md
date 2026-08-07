<img src="brand/mandalo-appicon.svg" width="72" align="left" alt="Mándalo">

# Mándalo

*¡Mándalo!* — send it. A fast, offline, git-native API client. Postman without the bloat, the forced account, or the cloud.

## Why

- **Offline, no account** — everything lives on your machine. No sign-in wall, no telemetry.
- **Nothing to publish** — a collection is a git repository of `.http` files, so any public repo already is one. `mandalo open owner/name`, or share `mandalo.dev/app?repo=owner/name`. It opens [read-only, after a review, with nothing running](docs/remote-collections.md).
- **Git-native** — requests are plain-text [`.http`](docs/http-format.md), [`.grpc`](docs/grpc-format.md), [`.ws` and `.mqtt`](docs/stream-formats.md) files in a [workspace directory](docs/workspace.md); environments and manifests are TOML. Diff them, review them, PR them.
- **Bring what you have** — import an [OpenAPI or Swagger specification](docs/openapi-import.md) or a [Postman collection](docs/postman-compatibility.md) with `mandalo import`; both report exactly what did not survive the trip.
- **Fast** — Tauri + Rust core. Instant startup, tiny footprint.
- **HTTP · GraphQL · gRPC · WebSocket · SSE · MQTT** — one workbench. gRPC compiles your `.proto` files at runtime (no `protoc` needed), and a [stream is a saved request](docs/stream-formats.md) like any other: it lives in a collection, carries named messages, and runs with `mandalo listen`.
- **Auth built in** — Bearer, Basic, API key (header or query).
- **Environments** — `{{variable}}` interpolation across URL, headers, body, and auth. Unresolved variables fail loud, never silently.

- **Runs in CI** — the same engine ships as `mandalo`, a command line runner with pretty/JSON/JUnit reporters, secret redaction and a credential scanner.

## Try it in two minutes

The repository ships a ready-to-run workspace at `examples/mock-workspace` —
collections for HTTP, GraphQL, gRPC, uploads (including a multipart form with
two files under one field) and realtime streams, plus two environments. The `hosted` environment
points at a public mock, so nothing has to run locally:

```bash
git clone https://github.com/De-Rus/mandalo
mandalo --workspace mandalo/examples/mock-workspace run mock --env hosted
```

Or open the canonical [Swagger Petstore](https://petstore3.swagger.io) spec —
it ships at `examples/petstore/openapi.json`. Opening that folder as a workspace
in the desktop app imports it automatically (19 requests under `pet/`, `store/`,
`user/`). From the CLI, import into any workspace:

```bash
mandalo --workspace ~/Mandalo import examples/petstore/openapi.json
```

Every compromise the import made — an OAuth flow it will not run, a body format
it had to pick — is printed, never swallowed.

Prefer everything on your machine? Start the local mock with `make mock-api` and
swap `--env hosted` for `--env local`. To browse the mock collection in a GUI,
open `examples/mock-workspace` as a workspace in the desktop app, or open it in
VS Code with the Mándalo extension installed — every request gets a Send lens
above it.

## Install

Three things ship from every `v*` tag, all on the same
[GitHub release](https://github.com/De-Rus/mandalo/releases/latest).

**Desktop app** — macOS `.dmg` (Apple silicon, Intel, or universal), Windows
`.msi`, Linux `.AppImage` or `.deb`. Once installed from a **published** GitHub
release, the app checks for updates on launch (and via **Check for updates…**)
and can install the next signed build itself. Draft releases do not count as
`latest`, so auto-update only kicks in after you publish the draft.

⚠️ **The desktop builds are unsigned, and macOS does not merely warn about
that.** There is no Apple Developer ID certificate behind this project yet, so
the `.dmg` is ad-hoc signed with no sealed resources. macOS treats a downloaded
copy as *broken*, not as untrusted: you get

> “mandalo is damaged and can’t be opened. You should move it to the Trash.”

and the usual right-click → Open escape does **not** appear. The dialog is about
the missing signature, not about the file being corrupt.

If you want to run it anyway, that is your decision to make knowingly — you are
choosing to trust a binary whose origin nothing has verified for you. Check the
download against the release's `SHA256SUMS` first, then remove the quarantine
flag by hand:

```bash
shasum -a 256 ~/Downloads/mandalo_0.1.0_aarch64.dmg   # compare with SHA256SUMS
xattr -d com.apple.quarantine ~/Downloads/mandalo_0.1.0_aarch64.dmg
```

If you would rather not do that — a reasonable position — use the CLI below, or
build from source with `pnpm tauri build`.

What would actually fix it: an Apple Developer ID certificate ($99/year) to sign
and notarize the `.dmg`, and an EV or Azure Trusted Signing certificate for the
Windows `.msi`. Both are purchases, not code. Until then Windows SmartScreen
also asks for a confirmation, though there the “More info → Run anyway” escape
does exist.

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
mandalo init ./my-api --name "Acme API"    # mandalo.toml + collections/ + environments/
mandalo run api --env staging --reporter junit > results.xml
mandalo send api auth.http#0 --env staging
mandalo ls
mandalo env list
mandalo scan --staged                     # git hygiene guard, exit 1 on a hit

mandalo open acme/collections              # somebody's public repo, read-only
mandalo open acme/collections --review-only
mandalo save-copy ~/work/billing           # make a read-only copy yours

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

## Shared values and yours

An environment file is meant to be committed, so every variable declares
whether its value is **shared** with the team. Two orthogonal questions:
`shared` says where the value lives, `secret` says how it is treated.

```toml
# environments/prod.toml — committed
schema_version = 1
name = "prod"

[vars.baseUrl]
value = "https://api.acme.com"   # shared — the common case stays a one-liner

[vars.devUrl]
shared = false                   # yours only; not masked

[vars.access_token]
secret = true                    # implies shared = false; masked and redacted
hosts = ["api.acme.com"]
```

A `secret = true` variable can never also be `shared = true`, and neither kind
has anywhere to put a `value` — the file format cannot express a value that is
not shared, so one cannot be committed by accident.

Everything that is not shared lives in one file, outside every workspace:

```toml
# ~/.config/mandalo/secrets.toml — 0600, never in any repository
schema_version = 1

[auth.github]
token = "gho_…"

[workspaces.550e8400-e29b-41d4-a716-446655440000.prod]
access_token = "…"
devUrl = "http://localhost:3000"
baseUrl = "http://localhost:8080"   # may override a shared value too
```

It is keyed by workspace **id**, so moving or re-cloning a workspace keeps its
values attached. `MANDALO_SECRETS_FILE` points it elsewhere.

```bash
pbpaste | mandalo env set-secret prod access_token   # never as an argument: ps is public
echo http://localhost:3000 | mandalo env set-local prod devUrl
mandalo env clear-secret prod access_token           # forgets the value, keeps the declaration
mandalo env get prod                                 # says which layer each value came from
```

**Precedence: `MANDALO_SECRET__<ENV>__<KEY>` → `secrets.toml` → the committed
value.** The exported variable wins because it is the more explicit and more
ephemeral act — the rule dotenv, direnv and compose all follow — and because it
is the layer CI injects: a stale local file that reached a build agent must
never shadow the credential the pipeline was given. `mandalo env get` names the
layer, so a value arriving from an unexpected one is visible rather than a
surprise.

A variable that is not shared and has no value anywhere fails before the
request leaves, naming the environment, the variable and every place a value
could come from. An empty value is never substituted. Every resolved secret is
scrubbed from `mandalo`'s own output (and emitted as `::add-mask::` under
GitHub Actions).

A workspace cloned from a colleague arrives with the declarations and none of
the values, which is the correct onboarding state — `mandalo env get` lists
what is still missing.

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
- Request history, cookie jar
- AI-assisted request generation
