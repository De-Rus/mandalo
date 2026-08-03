# Claude Code Context — Mándalo

Postman-alternative desktop API client. Tauri 2: the Rust side IS the network core (reqwest/tonic — no CORS, no proxy); React is a thin workbench over typed `invoke` calls.

## Principles

- **Offline-first, no account.** Nothing phones home.
- **Git-native.** Collections + environments = plain TOML files in the user's workspace dir (`~/Mandalo` default). Any sync/collab feature must reduce to git operations on those files.
- **Fail loud.** No fallbacks. Unresolved `{{vars}}`, invalid JSON, unsupported gRPC streaming — all return clear errors, never silent degradation.
- **Zero comments** in code; only non-obvious WHY / SAFETY / public-API docs.

## Layout

```
crates/core/src/    # mandalo-core — the product. No tauri, no clap.
  error.rs          # CoreError + stable code() per failure class
  capability.rs     # SecretStore / HostPolicy seams: EnvVarStore, NoSecrets, AllowAll, StrictPolicy
  runner.rs         # Runner: pre-script → send → post-script → tests → captures; run_suite
  request.rs        # HTTP + GraphQL send, auth application
  grpc.rs           # protox compile → prost-reflect DynamicMessage → tonic unary
  interpolate.rs    # {{var}} substitution, fail-loud
  workspace.rs      # environments as <workspace>/environments/<name>.toml
  collection.rs     # collections-as-dirs, folders-as-dirs, path confinement
  assertions.rs     # declarative tests + captures
  script.rs         # rquickjs pm.* sandbox
  postman.rs bundle.rs
crates/cli/src/     # mandalo-cli — binary `mandalo`
  main.rs redact.rs report.rs scan.rs style.rs
src-tauri/src/
  lib.rs            # thin adapter: #[tauri::command] wrappers, CoreError → String at the edge
src/
  lib/api.ts        # typed invoke wrappers — the IPC contract lives here
```

## Invariants

- All interpolation happens Rust-side, before auth is applied; the frontend only passes `vars`.
- Non-2xx HTTP responses are results, not errors. Transport failures are errors (strings).
- IPC payloads are camelCase over the wire (`serde(rename_all = "camelCase")`).
- gRPC is unary-only today; streaming methods must be rejected loudly, listed as disabled in the UI.
- Core returns `CoreError`; only the Tauri edge flattens it to `String`. GUI, CLI and CI must share the `Runner` so they cannot drift.
- Capabilities are injected, never read ambiently: secrets through `SecretStore`, egress through `HostPolicy`.

## Test & build

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm test          # vitest
pnpm build         # tsc + vite, the typecheck gate
pnpm tauri dev
```
