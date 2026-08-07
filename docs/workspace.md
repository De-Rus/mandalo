# The workspace

A Mándalo workspace is a directory you own. Everything in it is plain text you
can read, diff and commit; nothing about it is a database, and no file has a
format you cannot write by hand.

```
my-api/
├── mandalo.toml                  # the workspace manifest
├── collections/
│   └── acme-api/
│       ├── collection.toml       # the collection manifest
│       ├── health.http
│       └── auth/
│           ├── 1-login.http
│           └── 2-me.http
├── environments/
│   ├── local.toml
│   └── staging.toml
├── protos/                       # anything a .grpc request points at
└── files/                        # anything a `< ./file` body points at
```

Requests are [`.http`](http-format.md), [`.grpc`](grpc-format.md), [`.ws` and
`.mqtt`](stream-formats.md) files.
Folders are directories. The three TOML files are documented below.

## Creating one

```sh
mandalo init                    # here, named after the directory
mandalo init ./my-api --name "Acme API"
```

`init` writes `mandalo.toml`, creates `collections/` and `environments/`, and
registers the workspace so `--workspace <id>` finds it later. It is safe to run
in a directory that is already a workspace: the id is kept.

Every other command needs a workspace and says so if it does not find one:

> `/tmp/nope is not a Mándalo workspace — there is no mandalo.toml in it; create one with `mandalo init /tmp/nope``

`--workspace` accepts an absolute path, a path relative to the current
directory (`--workspace ./my-api`), or a registered workspace id.

## `mandalo.toml`

The workspace manifest. Three keys, all required:

```toml
schema_version = 1
id = "9f1c3f5e-3b8f-4a2b-9f5a-1c2d3e4f5a6b"
name = "Acme"
```

| Key | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Always `1` today. A file declaring anything else is refused rather than guessed at: `unsupported workspace schema_version 2 (expected 1)`. |
| `id` | string | Stable identity of the workspace, used by the registry (`~/.config/mandalo/workspaces.toml`) so a workspace keeps its place when the directory moves. Any unique string; `init` mints a UUID. |
| `name` | string | What the app and the CLI call it. Free text. |

A directory becomes a workspace when this file exists — that is the whole rule.

Optional share settings control how Export and Sync present the workspace to
tools that are not Mándalo:

```toml
[share]
format = "postman"   # or omit / "native" for Mandalo-only
dir = "postman"      # optional; directory for the generated mirror
```

| Key | Type | Meaning |
| --- | --- | --- |
| `format` | string | `native` (default) keeps Export as a Mándalo bundle and Sync as git of the workspace files only. `postman` makes Export write Postman v2.1 JSON, and makes Sync regenerate `<dir>/*.json` before the commit so the remote carries a Postman mirror alongside the native files. |
| `dir` | string | Relative directory for that mirror. Defaults to `postman`. |

The Export dialog writes this stanza when you pick **Postman**; Sync then
includes the generated files in the review list.

One optional stanza, written only by `mandalo open`, never by hand:

```toml
[remote]
label = "github.com/acme/collections"
url = "https://github.com/acme/collections"
commit = "0f1e2d3c4b5a…"
fetchedAt = 1754400000
```

Its presence makes the workspace **read-only** — it is somebody else's copy,
[opened from a link](remote-collections.md). Every write is refused with
`E_READ_ONLY` until `mandalo save-copy` turns it into a workspace you own.

## `collections/<slug>/collection.toml`

One per collection directory, same three keys:

```toml
schema_version = 1
id = "3a7d1c92-6f43-4b0e-b6c0-9d2f1a4e77c1"
name = "Acme API"
```

| Key | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Always `1`. |
| `id` | string | Stable identity, kept across renames, and carried by an exported bundle. |
| `name` | string | The display name. The **directory name** is the slug used on the command line (`mandalo run acme-api`); renaming the collection renames the directory. |

A directory under `collections/` with no `collection.toml` is not a collection.
It is listed as skipped rather than silently ignored:

> `skipped /…/collections/orphan/collection.toml: No such file or directory`

A slug may hold lowercase letters, digits and `-`.

## `environments/<name>.toml`

One file per environment. `name` inside the file is the name you pass to
`--env`; the file name matches it.

```toml
schema_version = 1
name = "local"

[vars]
baseUrl = "http://localhost:3000"
grpcUrl = "http://localhost:50051"
token = { secret = true, hosts = ["api.example.com"] }
devUrl = { shared = false }
```

`[vars]` holds one entry per variable. A variable declares two orthogonal
things: `shared` — whether its value lives in this committed file — and
`secret` — whether it is masked and redacted wherever it appears.

| Written | Meaning |
| --- | --- |
| `key = "value"` | Shared: the value is in the file, and is committed with it. |
| `key = { shared = false }` | Yours only. The value lives on this machine (`mandalo env set-local`), not in the file. |
| `key = { secret = true, hosts = [...] }` | A credential. Implies `shared = false`, and the value is masked in every view and scrubbed out of every message (`mandalo env set-secret`, or `MANDALO_SECRET__<ENV>__<KEY>`). `hosts` binds it: a request to any other host refuses to send it, and an empty list means "not bound yet". |

Only a shared variable has a `value` field, so a value that is not shared cannot
be committed by accident. Writing both is refused:

> `variable "token" is declared secret = true and also carries a value`

See the README's "Shared values and yours" for where the values that are not
shared are kept.

Variable names may hold letters, digits, `_`, `-` and `.`. Environment names may
hold letters, digits, `_` and `-`.

## What else can live here

Nothing else is required, but two directories are conventional because paths in
requests resolve against the workspace root, never against the file:

- `protos/` — what a `.grpc` request's `proto:` line points at.
- `files/` — what a `< ./files/payload.json` body points at.

A path that leaves the workspace is a hard stop, not a convenience: a collection
is shared, untrusted configuration.

## Moving one somewhere else

`mandalo export bundle.json` writes the whole workspace — every collection,
folder, request and environment declaration — into one JSON file, and
`mandalo import bundle.json` reproduces it in an empty workspace. Secret
*values* are never in a bundle; their declarations are. An import that cannot be
completed writes nothing at all.

Git works too, and is the better answer for anything ongoing:
`mandalo git-hygiene --install-hook` sets up a `.gitignore` and a pre-commit
credential scan, and `mandalo sync` commits, rebases and pushes.
