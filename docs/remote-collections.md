# Opening somebody else's collection

A collection is a directory of `.http` and `.grpc` files in a git repository. So
any public repository is already a publishable collection — there is nothing to
export, nothing to upload, and no account anywhere. Point Mándalo at it and it
opens.

It opens **read-only**, in a workspace of its own, and nothing in it runs until
you send a request yourself.

## What you can point at

| Form | Example |
| --- | --- |
| A public GitHub repository | `acme/collections` |
| …on a branch | `acme/collections#next` |
| …a subdirectory of one | `acme/collections/apis/billing` |
| …both | `acme/collections/apis/billing#next` |
| The URL from the address bar | `https://github.com/acme/collections` |
| …including the browsable tree URL | `https://github.com/acme/collections/tree/next/apis` |
| A clone URL | `https://github.com/acme/collections.git` |
| The short scheme | `github:acme/collections` |
| A raw URL into a repository | `https://raw.githubusercontent.com/acme/collections/main/apis` |
| A single Mándalo bundle *(desktop only)* | `https://acme.dev/team.json` |

```sh
mandalo open acme/collections
mandalo open acme/collections/apis#next ./billing
mandalo open acme/collections --review-only     # print the review, open nothing
```

In the app: **workspace ⋯ → Browse shared collection…**

### The link

`https://mandalo.dev/app?repo=acme/collections` opens the review for that
repository in the browser. That is the whole sharing mechanism: put the link in
a README and anyone with a browser can read your collection. `?bundle=<url>`
does the same for a bundle URL.

The link opens the **review**, never the collection and never a request. The
parameter is taken out of the address bar once it has been read.

## What you are shown before it opens

The same shape as the review before an export or a sync — say what will happen,
then do exactly that:

- how many requests, collections, environments, files and bytes;
- **every host the collection would contact**, deduped and sorted, and separately
  every URL whose host is a `{{variable}}` you would be the one to set;
- **every environment**, with how many variables it declares, how many carry a
  value in the file, and how many you would have to supply yourself;
- **every script it carries** — which request, which hook, how many lines — with
  the plain statement that none of them run on opening;
- **anything the credential scanner found** in the files that were fetched;
- anything that was **not taken**, and why.

The review carries a `token` that pins it to the exact bytes that were fetched.
Opening hands the token back; if it no longer describes those bytes, nothing is
written. The bytes are already in hand, so opening never fetches again — a second
fetch could return something else.

## Read-only, and how to stop being read-only

A remote workspace's `mandalo.toml` carries where it came from:

```toml
schema_version = 1
id = "9f1c3f5e-…"
name = "github.com/acme/collections"

[remote]
label = "github.com/acme/collections"
url = "https://github.com/acme/collections"
commit = "0f1e2d3c4b5a…"
fetchedAt = 1754400000
```

That stanza is what makes it read-only, and it survives every rewrite of the
file. While it is there, the engine refuses **every** write: saving a request,
creating, renaming or deleting a collection or folder, moving a request, saving
or deleting an environment, and adding the sample collection. They fail with
`E_READ_ONLY` and the same sentence — *save a copy of it to make changes*.

Sending requests is not a write, and works normally.

```sh
mandalo --workspace ./billing save-copy ~/work/billing
```

In the app: the **Save a copy locally** button in the read-only bar. Same files,
new identity, no `[remote]` stanza, writable. The original is untouched.

## Why it is treated as hostile

You did not get this from a colleague — you clicked a link. So:

- **Nothing runs on open.** The files are parsed, never executed. A script runs
  when you send the request it belongs to, and not before.
- **Every fetch goes through the egress guard**, on the first URL and on every
  redirect hop, exactly like a request you send. A repository cannot redirect
  Mándalo at a host the policy refuses. On the desktop the fetch happens in the
  engine, never in the webview.
- **Everything is bounded.** At most 400 files, 512 KiB per file, 4 MiB in total,
  10 directories deep. A file over the per-file cap is never even requested — the
  size comes from the listing.
- **Only workspace files are taken.** `.toml`, `.http`, `.rest`, `.grpc`, `.ws`,
  `.mqtt`, `.proto`, `.json`, `.txt`, `.graphql`, `.gql`, `.md`, `.xml`, `.csv`.
  Anything else is listed as skipped. So are dotfiles and dot directories, and so
  is any file that holds one machine's values (`.env`, `secrets.toml`,
  `*.local.toml`).
- **Paths cannot climb out.** A path with `..`, an absolute path, or one that
  would land outside the workspace is refused, not sanitised.
- **A collection that does not load whole does not load at all.** If any file
  arrives and will not parse, nothing is opened.

### Credentials never travel

A remote environment arrives as *declarations*. The format has no way to carry a
value for anything that is not shared: `VarDef::Secret` and `VarDef::Local` have
no `value` field at all. A shared value — a base URL — is in the file by design,
and is exactly what the host list is built from.

Nothing in this path prompts for a credential, and nothing stores one.

A URL with a credential in it (`https://<token>@github.com/…`, which people do
paste) is **refused**, and the URL is never echoed back into the error, a log or
the workspace registry.

## Private repositories are a different thing

If you can read a private repository, you do not want a preview — you want to
work in it and push back. That is not this feature; it already exists.

```sh
mandalo login                 # device flow; the token lands in ~/.config/mandalo/secrets.toml
mandalo clone https://github.com/acme/private.git ./private
```

That gives you a normal workspace you own, with `mandalo sync` for commit, pull
and push.

GitHub answers **404** both for a repository that does not exist and for a
private one you are not signed in to — it will not confirm that a private
repository is there. So `mandalo open` says both possibilities, and does not
assert which:

> GitHub did not return acme/private. It may not exist, or it may be private —
> GitHub answers the same way for both when nobody is signed in. If it is
> private, a read-only preview is not what you want anyway: sign in to GitHub and
> clone it into a workspace of your own, which you can edit and push back to.

### The browser never takes a token

In the browser, public repositories only. `raw.githubusercontent.com` and
`api.github.com` both send CORS headers, so the page reads a public repository
directly — no proxy of ours, nothing in the middle. Every request is sent with
`credentials: "omit"`.

When a repository turns out to be private, the browser says it needs the desktop
app, and stops. There is no token field in the web UI, no token accepted in the
URL, and nothing stored in browser storage. A web page is not a safe place for a
credential that can read every repository you own — one cross-site scripting bug
and it is gone.

A single-file bundle is desktop-only for the same reason the importer is: the
browser build has no bundle reader.

### The scanner is honestly weaker in the browser

The desktop runs the full scanner: named credential patterns, plus entropy and
decoded payload guesses. The browser runs the **named patterns only** — the same
13 rules, generated from `crates/core/src/scan.rs` so they cannot drift — and the
review says so rather than letting an empty result read as a clean bill of
health.

## The sample collection

The one collection everybody can always get back:

```sh
mandalo open …                    # somebody else's
```

In the app, on every host: **workspace ⋯ → Add sample collection**. It copies
the shipped sample into the current workspace under a free slug (`mock`, then
`mock-2`, `mock-3`…) and touches nothing already there. Supporting files the
sample's requests point at (`protos/`, `files/`) come along; an environment is
written only when the workspace has none by that name.

Both hosts write the same tree: the desktop inlines
`examples/mock-workspace` at build time (`crates/core/build.rs`), the browser
globs the same directory with the same extension list, and a test on each side
pins both to the files on disk.
