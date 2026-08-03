# Signing in to GitHub

Mándalo pushes and pulls your workspace with git, so it needs a GitHub token. It
obtains one **without a server of ours and without ever seeing your password**:

- no Mándalo backend is involved — the browser talks to github.com, the app talks
  to github.com, and nothing passes through us;
- no embedded webview ever renders a GitHub login form — that is the pattern
  credential phishing uses, so the app hands the URL to your real browser and
  waits;
- the token lands in your OS keychain, never in a file inside the workspace.

## The device flow

1. The app asks GitHub for a code (`POST https://github.com/login/device/code`).
2. It shows you an eight-character code and opens
   `https://github.com/login/device` in your browser.
3. You type the code there and approve the scopes, in GitHub's own UI.
4. Meanwhile the app polls `POST https://github.com/login/oauth/access_token`
   until GitHub hands over a token.

There is no client secret anywhere in this: the device flow is designed for
native apps precisely because a shipped binary cannot keep one.

### Polling and backoff

GitHub dictates the pace. The first request comes back with an `interval` (five
seconds today) and the app waits that long between polls — including **before**
the first one, since you have not typed the code yet.

While you are still deciding, GitHub answers `authorization_pending` and the app
keeps waiting. If it ever answers `slow_down`, the interval widens — to the
number GitHub names, or by five seconds when it names none — and it never narrows
again. Ignoring that is how a client gets rate-limited.

Terminal answers stop the flow with a plain-language message: the code expired
(`expired_token`), you cancelled on github.com (`access_denied`), or GitHub no
longer recognises the attempt (`incorrect_device_code`).

## Scopes

Mándalo asks for **`repo`**.

`repo` is the narrowest classic OAuth scope that can push to a **private**
repository — there is no read/write-code-only scope for OAuth Apps. If your
collections live in a public repository you can ask for less:

```bash
mandalo login --public-only    # public_repo instead of repo
```

Deliberately **not** requested:

- `workflow` — Mándalo pushes request and environment files, never
  `.github/workflows`. A push that touched a workflow file will be refused by
  GitHub, which is the correct outcome rather than quietly holding more power.
- `admin:org`, `delete_repo`, `gist`, `user` — none of them are needed to sync
  files.

## Personal access tokens

If you would rather not use the browser flow at all — many developers would not —
pipe a token in:

```bash
pbpaste | mandalo login --with-token          # macOS
mandalo login --with-token < token.txt        # anywhere
```

It is read from **stdin, never from an argument**: `ps` shows every process's
argv to every user on the machine. The token is checked against
`GET https://api.github.com/user` before it is stored, so a bad paste fails
immediately instead of at your next push.

A fine-grained PAT works too, and is the tighter choice: scope it to the single
repository that holds your workspace, with **Contents: read and write**.

## Where the token lives

The OS keychain, under service `com.drus.mandalo`, account `auth/github/token`.
It is deliberately **not** scoped to a workspace — one GitHub identity serves all
of them.

The moment a token exists it is registered with the process redactor, so it
cannot surface in an error message, a log line, a report or an IPC string. The
desktop app never sends the token to the frontend either: the sign-in commands
return your GitHub identity, and the git layer reads the keychain Rust-side.

```bash
mandalo whoami          # who the stored token belongs to
mandalo logout          # forget it
```

## The OAuth App

The shipped client id is already configured:

```
Ov23liXEjTIMDJe2lT2Q
```

A client id is a **public** identifier — like Google's or Slack's. It carries no
secret, the device flow has no client secret, and it is committed to this
repository on purpose.

### Overriding it for a fork or a self-hosted build

Point at your own OAuth App without patching a line of source. Resolution order:

1. an explicit argument — `mandalo login --client-id Ov23li…`, or the
   `clientId` parameter of `github_start_login`;
2. the `MANDALO_GITHUB_CLIENT_ID` environment variable at run time;
3. `MANDALO_GITHUB_CLIENT_ID=Ov23li… cargo build`, which bakes it in at compile
   time;
4. otherwise `github_auth::MANDALO_CLIENT_ID` in
   `crates/core/src/github_auth.rs`.

To register your own: GitHub → Settings → Developer settings → OAuth Apps → New
OAuth App. Leave the callback URL empty — the device flow does not use one — and
**tick "Enable Device Flow"**. Without that box, GitHub answers
`device_flow_disabled` and Mándalo says exactly that.

A build that ships without an OAuth App at all (set the constant to
`CLIENT_ID_PLACEHOLDER`) refuses to start a sign-in and tells the user to set
`MANDALO_GITHUB_CLIENT_ID`, rather than sending a made-up id to GitHub.

### Checking the app is live

The handshake needs no user interaction and no secret, so it can be verified
directly. It is off by default so CI stays offline:

```bash
MANDALO_GITHUB_LIVE=1 cargo test -p mandalo-core --test github_auth \
  the_shipped_client_id_really_starts_a_device_flow
```

A pass proves both that the client id is registered and that Device Flow is
enabled on it.
