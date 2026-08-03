# Claude Code Context — Mándalo

Postman-alternative desktop API client. Tauri 2: the Rust side IS the network core (reqwest/tonic — no CORS, no proxy); React is a thin workbench over typed `invoke` calls.

## Principles

- **Offline-first, no account.** Nothing phones home.
- **Git-native.** Collections + environments = plain TOML files in the user's workspace dir (`~/Mandalo` default). Any sync/collab feature must reduce to git operations on those files.
- **Fail loud.** No fallbacks. Unresolved `{{vars}}`, invalid JSON, unsupported gRPC streaming — all return clear errors, never silent degradation.
- **Zero comments** in code; only non-obvious WHY / SAFETY / public-API docs.

## Layout

```
src-tauri/src/
  lib.rs          # command registry
  request.rs      # HTTP + GraphQL send, auth application
  grpc.rs         # protox compile → prost-reflect DynamicMessage → tonic unary
  interpolate.rs  # {{var}} substitution, fail-loud
  workspace.rs    # environments as <workspace>/environments/<name>.toml
src/
  lib/api.ts      # typed invoke wrappers — the IPC contract lives here
  lib/collection.ts # request persistence seam (localStorage now, git files later)
```

## Invariants

- All interpolation happens Rust-side, before auth is applied; the frontend only passes `vars`.
- Non-2xx HTTP responses are results, not errors. Transport failures are errors (strings).
- IPC payloads are camelCase over the wire (`serde(rename_all = "camelCase")`).
- gRPC is unary-only today; streaming methods must be rejected loudly, listed as disabled in the UI.

## Test & build

```bash
cargo test --manifest-path src-tauri/Cargo.toml
pnpm test          # vitest
pnpm build         # tsc + vite, the typecheck gate
pnpm tauri dev
```
