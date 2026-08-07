# Export and sync: choose, then review

Both operations send part of your workspace somewhere else — a file you will
attach to a message, or a git remote your team reads. Neither one runs on its
own. You choose what goes, Mándalo shows you exactly that, and only then does
anything leave the machine.

## Two phases, always

```
plan_export(workspace, selection) -> ExportPlan   # nothing is written
run_export (workspace, selection, plan.token, path, force)

plan_sync(workspace, selection, branch) -> SyncPlan   # nothing is committed
run_sync (workspace, selection, plan.token, message, auth, force)
run_branch_push(workspace, selection, branch, plan.token, message, auth, force)
```

The `run_*` calls take the `token` the plan produced. They re-plan, compare, and
refuse with `E_CONFLICT` when it no longer matches — because the workspace
changed, or because no plan was made at all. There is no way to write, commit or
push without having produced the preview first.

## Choosing what goes

### Export

Everything is the default. Narrowing is opt-in.

```bash
mandalo export bundle.json                                   # the whole workspace
mandalo export api.json --collection acme-api
mandalo export users.json --collection acme-api --folder users
mandalo export one.json  --request acme-api:users/list.http
mandalo export envs.json --env staging --env prod
mandalo export collection.json --format postman --collection acme-api
```

`--format postman` writes a Postman v2.1 collection (or a single environment).
Without `--format`, Export reads `[share] format` from `mandalo.toml` (default:
Mándalo bundle). Postman export is one file per collection or environment.

When `[share] format = "postman"`, `mandalo sync` regenerates `postman/*.json`
into the working tree before planning the commit, so the remote stays usable
from Postman without replacing the native Mandalo files.
- `--folder` and `--request` take `<collection>:<path>`. The bare form is
  accepted only when exactly one `--collection` was given, since that is the
  only time it cannot be misread.
- `--request` accepts a file (`users/list.http`, every request in it) or one
  request's full address (`users/list.http#1`).
- `--all` says "the whole workspace" out loud and refuses to be combined with a
  filter.
- Naming a collection, folder, request or environment that does not exist is an
  error, never a silently empty export.

### Sync

Git already knows which files changed. The selection only decides which of them
this commit carries.

```bash
mandalo sync -m "add the orders endpoints"
mandalo sync -m "just the users work" --only collections/acme-api/users
mandalo sync -m "not that one yet"    --except environments/staging.toml
```

A file left out is not dropped: it is never staged, so it stays modified in the
working tree and goes in whenever you next include it. A file you had already
`git add`ed but did not select is not committed either — the index is put back in
step with `HEAD` before the chosen paths are staged.

## What the preview tells you

`ExportPlan`:

- **included** — the collections by name, every request path in them, the
  environments, the request count, and the size in bytes.
- **excluded** — how many secret values and how many local values are *not* in
  the file (said positively: the mechanism worked), which collections and
  environments were left out, and how many requests inside the chosen
  collections were not selected.
- **findings** — what the credential scanner found in the bundle that is about to
  be written: rule, line, and an excerpt that shows the first few characters and
  the length, never the credential.

`SyncPlan`:

- **action** — `commit`, `commitAndPush`, `push`, `pull`, `branchAndPush` or
  `nothing`. You are told which one before you agree to it.
- **remote** and **branch** — the remote URL any password is stripped out of, and
  the branch that will be pushed. `targetBranch` is the new branch a pull-request
  push would create.
- **files** — every changed path, what changed about it, and whether it is in
  this commit.
- **ahead** / **behind** — as of the last fetch. Planning never touches the
  network on its own.
- **identity** — who the commit will be attributed to, and whether that is the
  fallback because the repository has no `user.name`.
- **findings** — the scanner over exactly the selected files, nothing else.

## What blocks, and what only warns

Blocks, with nothing written or pushed:

- any credential finding in what would be written or committed,
- a token that does not match a fresh plan,
- a collection, folder, request, environment or path that does not exist,
- an unreadable workspace file, or an unresolved merge conflict.

Warns, and carries on:

- the fallback commit identity,
- the counts of excluded secrets, files and collections,
- a workspace with no remote (the sync commits and says so).

`force` is the only override, it covers the scanner and nothing else — it never
force-pushes and never rewrites history — and it is passed per call. There is no
setting that turns the check off for next time.

## On a terminal, and in CI

```bash
mandalo export bundle.json            # prints the plan, asks, waits
mandalo export bundle.json --yes      # unattended
mandalo sync -m "..." --yes --force   # unattended, past a finding you accepted
```

Without a terminal to answer on and without `--yes`, both commands refuse rather
than assume consent. With `--yes` but with findings present, they still refuse
unless `--force` is also given.

Every line of this output goes through the redactor, so a token that reached the
process cannot appear in a plan, a prompt or an error.

## Secrets are not the thing at risk here

A secret or local variable's value never lives in the workspace — it is in
`$HOME/.config/mandalo/secrets.toml`, outside anything a file-based export or a
git commit can reach. The environment documents carry declarations and shared
values only, which is why the plan can tell you the count of what it is not
carrying.

What *can* leak is a credential someone typed straight into a request, a header
or a script. That is what the scanner reads, and that is what blocks.

## What the scanner reads

Every file, including dotfiles — `.env` and `.secrets.toml` commit like anything
else, so they are checked like anything else. Only `.git/` is skipped, because
nothing inside it is ever committed. A file that is not valid UTF-8 is read
anyway, lossily, and a file that cannot be read at all is reported rather than
passed over: a check that gives up on what it cannot read is a check an attacker
turns off with one stray byte. Inside a binary file only the named credential
patterns apply — entropy and look-alike-letter guesswork mean nothing there.

`mandalo scan --staged` reads the **staged** content, not the working copy.
Staging a token and then editing the file back is exactly the shape a pre-commit
hook has to catch.

It looks for named credential shapes (AWS, GitHub, GitLab, Stripe, Slack, npm,
Google, JWTs, private keys, `user:password@` in a URL), the same shapes after
undoing look-alike letters, base64-wrapped copies of them, a token split across
two lines or continued with `\`, and — inside a quoted value or the right hand
side of an assignment only — a high-entropy run or a long value under a name
like `password`, `token` or `api_key`. It deliberately says nothing about
request paths, URLs, UUIDs, identifiers, media types or placeholders such as
`{{token}}`, `$ACME_API_KEY` and `changeme`: it blocks an export and a commit,
so a false positive is not noise — it is a workspace nobody can ship, and a
habit of reaching for `--force`.
