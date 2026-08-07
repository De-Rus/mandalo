# Mándalo for VS Code

Mándalo keeps API collections as plain text files in your repo: `.http` / `.rest` for HTTP
and GraphQL, `.grpc` for gRPC. This extension drives the files that are already in your
workspace — there is no sync, no account, no cloud. Open the repo, and your requests are
right there next to the code they call.

Works in VS Code and in Cursor (no proprietary APIs are used).

## What you get

- **`▶ Send` above every request** — one file holds many requests, separated by lines
  starting with `###`. Each block gets its own `▶ Send` and `▶ Send with env…` CodeLens on
  its separator line, so sending is a click where you are already reading. Syntax
  highlighting comes with it, for the separator and its name, the method and URL,
  headers, `{{vars}}`, `@var` definitions and `{% %}` script blocks.
- **Collections view** in the Activity Bar — workspaces → collections → folders → requests,
  where every `###` block is its own entry, labelled with its name and described by its
  method (or `GraphQL` / `gRPC`). It refreshes when the files change, so hand-editing the
  text is the whole workflow.
- **`▶ Run all`** on a `collection.toml` manifest.
- **Response panel** — status chip, duration, size, and Body / Headers / Tests / Captures tabs.
- **Native Testing panel** — collections become suites, every `###` block becomes a test,
  and each assertion becomes a child item with its own failure message.
- **Environment picker** in the status bar, persisted per workspace.

- **Diagnostics while you type** — `.http`, `.rest` and `.grpc` files inside a collection are
  parsed on every keystroke; a format error is squiggled on its own line with the same
  sentence the CLI would print, and a `{{var}}` the selected environment does not define is
  a warning with an **Add "name" to environment** quick fix.

## How requests are sent

HTTP and GraphQL requests are sent by the **in-process engine**, so the extension alone is
enough to send: no CLI, no install. gRPC needs HTTP/2 trailers that the editor's Node runtime
cannot read, a `< ./file` body needs the workspace on disk, and a `#name` address needs the
CLI's own name matching — those three escalate to the **`mandalo` CLI**, bundled with this
extension or pointed at by you. Every send logs which engine ran it and why to the **Mándalo**
output channel.

The extension therefore carries its own reader for the text formats, for the tree, the
CodeLens positions, the diagnostics and that engine. That reader is **interim** — it is to
be replaced by a WASM build of `crates/core`, the same pattern the repo already uses for
gRPC. Until then the Rust parser stays the reference and a parity suite proves it: the same
corpus is parsed by both readers and must agree on every request, name, method, URL, header
and body, down to the wording of a parse error.

Platform builds of the extension (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`,
`win32-x64`) **ship that binary inside the VSIX**; the Marketplace serves the right one
automatically. The binary is resolved in this order, and the choice is logged to the
**Mándalo** output channel:

1. the `mandalo.cliPath` setting, when you set one — it always wins;
2. the binary bundled at `<extension>/bin/mandalo`;
3. `mandalo` on your `PATH`.

If none exists and something genuinely needs the CLI, the error names your platform, says the
bundled binary is missing, and offers **Download from releases**, **Set path…** and
**Open output**. It never claims the CLI is installed when it is not.

When a `mandalo.cliPath` binary reports a different version from the extension, you get one
warning per session naming both versions, because their JSON contracts may differ.

`mandalo.executionMode` picks the engine: `auto` keeps HTTP and GraphQL in the editor and
escalates the rest, `in-process` refuses to shell out, `cli` always does. When something
genuinely needs the CLI and no binary exists anywhere, the error names the reason, the
missing binary and the two ways to supply one.

## Form-data with several files

A multipart field that posts more than one file under the same name is one line
with several `< ./path` references — the same shape a browser sends for
`<input type="file" multiple>`:

```http
### Upload
POST {{baseUrl}}/body/multipart
Content-Type: multipart/form-data

attachments = < ./files/alpha.txt < ./files/beta.txt
```

Repeating the field name on later lines still reads and folds into the same
field. Sending still goes through the CLI (multipart reads workspace files off
disk).

## Try it on the example workspace

The [Mándalo repository](https://github.com/De-Rus/mandalo) ships a ready-to-run
workspace at `examples/mock-workspace`: collections for HTTP, GraphQL, gRPC and
uploads, and a `hosted` environment that needs no local server. Clone the repo,
open that folder in VS Code, and every request in it gets a Send lens above it.

## Installation

From the Marketplace, or from a local build:

```bash
pnpm install
pnpm run compile
npx @vscode/vsce package --no-dependencies
code --install-extension mandalo-0.1.0.vsix
```

## Settings

| Setting | Default | What it does |
| --- | --- | --- |
| `mandalo.cliPath` | *(empty)* | Path to the CLI. Empty uses the bundled binary, then `PATH`. Setting it always wins. |
| `mandalo.executionMode` | `auto` | Which engine runs a request: `auto`, `in-process` or `cli`. |
| `mandalo.diagnostics.enabled` | `true` | Validate `.http`, `.rest` and `.grpc` files while you type. |
| `mandalo.codeLens.enabled` | `true` | Show `▶ Send` / `▶ Send with env…` above every `###` block. |
| `mandalo.timeoutMs` | `120000` | How long to wait for a CLI invocation. |

## Commands

All under the **Mándalo** category: Send Request · Send Request with Environment… ·
Resend Last Request · Run Request Tests · Run Collection · Run Folder ·
Select Environment · New Request · New Collection · Refresh ·
Open Workspace in Mándalo · Show Log.

### Keyboard shortcuts

| Shortcut | Command |
| --- | --- |
| `Ctrl+Alt+R` / `Cmd+Alt+R` | Send the request under the cursor |
| `Ctrl+Alt+E` / `Cmd+Alt+E` | Send it with a different environment |
| `Ctrl+Alt+L` / `Cmd+Alt+L` | Resend the last request, in the environment it was sent with |
| `Ctrl+Alt+Shift+E` | Select the environment |

## The on-disk layout it reads

```
<workspace>/
  mandalo.toml                  # schema_version, id, name
  environments/<name>.toml      # name + [vars]
  collections/<slug>/
    collection.toml             # schema_version, id, name
    <folder>/<file>.http        # HTTP + GraphQL requests
    <folder>/<file>.grpc        # gRPC requests
```

TOML is now only the manifests and the environments. Requests are text.

### Addressing a request

One file holds many requests, so a request is addressed as **`<path>#<index>`**, zero-based
in file order — `auth/login.http#0`, `auth/login.http#1`, `grpc/mock.grpc#0`. That is exactly
what the CLI takes, what the tree and the Testing panel key off, and what each `▶ Send` lens
carries:

```bash
mandalo send acme-api auth/login.http#1 --workspace . --reporter json
```

A `###` block that holds only comments and `@var` lines declares no request, so it never
consumes an index — a file header cannot shift the numbering of what follows.

### `.http`

The de-facto REST Client / httpYac / JetBrains dialect:

```http
@host = api.example.com

### Login
POST https://{{host}}/auth/login
Content-Type: application/json

{ "user": "ada" }

> {%
pm.environment.set("token", pm.response.json().token);
%}

### Get profile
GET https://{{host}}/me
Authorization: Bearer {{token}}
```

Also understood: `#` and `//` comments, the `# @name x` metadata comment, `< ./payload.json`
file bodies and `< {% … %}` pre-request scripts. A request carrying an
`X-REQUEST-TYPE: GraphQL` header is a GraphQL request — its body is the document, then a
blank line, then a JSON variables object.

### `.grpc`

Deliberately rhymes with `.http`. The request line is `<target>/<package.Service>/<Method>`,
and the header lines are gRPC metadata except the reserved, repeatable `proto:` key:

```
### Say hello
{{grpcUrl}}/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: mandalo

{ "text": "hola", "n": 21 }
```

Same `###` separators, same comments, same `@var` lines, same `> {% %}` / `< {% %}` scripts.

## Development

```bash
pnpm install
pnpm run compile           # esbuild bundle + tsc --noEmit
pnpm run watch             # rebuild on save
pnpm run test:unit         # vitest — parser, scanner, CLI adapter, diagnostics
pnpm run test:integration  # @vscode/test-cli — runs a real VS Code against fixtures/
pnpm test                  # both
```

`fixtures/workspace/` is a real Mándalo workspace used by both test layers; the
integration run opens it as the VS Code workspace folder.

`test/unit/cli.e2e.test.ts` and `test/unit/parity.test.ts` shell out to the **real** `mandalo`
binary. The parity suite runs the same collection through both engines and asserts identical
test names, verdicts, details and captures — it is what stops the in-process engine drifting
away from the Rust one. Get a binary the same way the extension does:

```bash
node scripts/fetch-cli.mjs      # downloads the release asset, or builds it with cargo
```

It lands in `editor-extension/bin/`, which is gitignored and is also what a platform VSIX
ships. Without one, those suites skip with a message naming the command; set
`MANDALO_REQUIRE_CLI=1` (as CI does) to turn that skip into a failure instead.

`src/engine/prelude.generated.ts` is generated from `crates/core/src/script.rs` by
`scripts/gen-prelude.mjs` and committed. CI regenerates it and fails on a diff, so the script
sandbox cannot drift from the Rust one.

## Packaging

```bash
node scripts/fetch-cli.mjs
npx @vscode/vsce package --target darwin-arm64 --no-dependencies
```

`vscode:prepublish` regenerates the prelude and fetches the CLI for the host platform, so a
plain `vsce package --target <host platform>` is self-sufficient. Cross-platform VSIXs are
built by `.github/workflows/extension.yml` on a `v*` tag, which also attaches them to the
GitHub release and publishes to the Marketplace when a `VSCE_PAT` secret exists.

## License

MIT
