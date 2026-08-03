# Mándalo

*¡Mándalo!* — send it. A fast, offline, git-native API client. Postman without the bloat, the forced account, or the cloud.

## Why

- **Offline, no account** — everything lives on your machine. No sign-in wall, no telemetry.
- **Git-native** — collections and environments are plain TOML files in a workspace directory. Diff them, review them, PR them.
- **Fast** — Tauri + Rust core. Instant startup, tiny footprint.
- **HTTP · GraphQL · gRPC** — one workbench. gRPC compiles your `.proto` files at runtime (no `protoc` needed).
- **Auth built in** — Bearer, Basic, API key (header or query).
- **Environments** — `{{variable}}` interpolation across URL, headers, body, and auth. Unresolved variables fail loud, never silently.

## Stack

Tauri 2 · Rust (reqwest, tonic, prost-reflect, protox) · React 19 + TypeScript + Vite · zustand

## Develop

```bash
pnpm install
pnpm tauri dev
```

## Test

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust core
pnpm test                                          # frontend (vitest)
```

## Roadmap

- GitHub login (Supabase Auth, PKCE) + commit/push collections to a shared repo — collaboration is just git
- Request collections persisted as workspace files (currently localStorage)
- gRPC streaming, server reflection
- Request history, cookie jar, OpenAPI import
- AI-assisted request generation
