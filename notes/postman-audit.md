# Mándalo vs Postman — audit and plan

Produced by a twelve-agent panel: a capability inventory read off the code, five
independent audit lenses (daily loop, feature parity, first five minutes, power
user/DX, trust and collaboration), a skeptic verifying every finding against the
source, and a synthesis. Findings that could not be confirmed in the code were
dropped before this document.

## The thesis

Someone picks Postman today because Mándalo's desktop app — the surface everyone installs first — cannot do the things a working day is made of, even though the Rust engine underneath already can. The engine ships git sync, GitHub auth, a suite runner, inherited auth, credential scanning and a pre-commit hook; the frontend calls almost none of it (git_status, plan_sync, run_sync, run_branch_push, git_clone, run_suite: zero callers in src/; ensure_git_hygiene and scan_workspace: wrapped, never called). Worse, the gap is not merely absence — it is destructive. A Postman-imported request's inherited auth is downgraded to `none` by an autosave 500 ms after you touch any field, deleting credentials from a git-tracked file; saved stream messages lose their payload on write and crash the workbench on read, reproducibly, using the shipped example workspace. Meanwhile the everyday gestures are missing (no duplicate, no move, no curl in or out, no history, URL and Params are unbound), the first ⌘N on a fresh install is a red error, and behind any corporate TLS-inspecting proxy every HTTP send fails while gRPC in the same binary succeeds. Closing it is mostly wiring and papercuts, not new engineering: stop the two data-loss bugs, make TLS trust the OS, make the first run and the first variable work, wire the git commands the product's entire thesis depends on, and give the app the four gestures (duplicate, move, curl, capture-a-value) that a developer performs a dozen times a day. Do that and the pitch — same engine, plain files, no cloud, reviewable in a pull request — becomes true in the app and not just in the CLI.

## Do now

### 1. Stop the two silent data-loss bugs: stream messages and inherited auth

*hours · app*

**Why.** Both destroy user data on an autosave the user never asked for — a saved WebSocket payload vanishes on write and crashes the workbench on read, and every Postman-imported request loses its credentials 500 ms after you touch a field.

**Approach.** Two fixes, one theme, both shippable today. (a) Streams: write the adapter that was never written — toStreamMessage(SavedMessage): StreamMessage and fromStreamMessage in src/lib/stream.ts (text → payload; publish → payload + topic + qos + retain; subscribe/unsubscribe/binary/ping have no StreamMessage representation and must be rejected loudly at save time, not flattened), called at src/lib/collection.ts:116 and :148. Guard src/components/stream/Composer.tsx:9-10 so a chip that cannot be converted renders disabled with its reason instead of `switch (message.kind)` on undefined. Add #[serde(deny_unknown_fields)] to StreamMessage in crates/core/src/collection.rs:130-144 so a future drift is a loud deserialisation error. Add a round-trip test loading examples/mock-workspace/collections/mock/streams/chat.ws and sensors.mqtt through toDraft asserting every chip has a payload — it fails today. (b) Inherited auth, step one only: replace `default: return base` in toAuthDraft (src/lib/collection.ts:222-224) with an explicit `case "inherited"` that sets a draft flag authUnrepresentable, and have doPersist in src/store/collection.ts refuse to autosave any draft carrying it, showing the standard error notice ('this request inherits auth from its collection; Mándalo cannot edit that yet'). ~30 lines that stops credentials being deleted from disk by an app the user never asked to do that.

Files: `/Users/d.rus/Projects/mandalo/src/lib/stream.ts`, `/Users/d.rus/Projects/mandalo/src/lib/collection.ts`, `/Users/d.rus/Projects/mandalo/src/components/stream/Composer.tsx`, `/Users/d.rus/Projects/mandalo/crates/core/src/collection.rs`, `/Users/d.rus/Projects/mandalo/src/store/collection.ts`

### 2. Make TLS trust match the OS, and name TLS failures as TLS failures

*hours · core*

**Why.** Behind any corporate TLS-inspecting proxy or private CA every HTTP send fails while curl works — and gRPC in the same binary succeeds, because tonic is the only client built with native roots.

**Approach.** One-line first: change reqwest in crates/core/Cargo.toml:19 from `rustls-tls` to `rustls-tls-native-roots`, and tokio-tungstenite:30 from `rustls-tls-webpki-roots` to `rustls-tls-native-roots`, matching the `tls-roots` already pinned on tonic at :21. Ship that alone if nothing else — it resolves the majority of cases and removes the self-inconsistency. Then add CoreError::Tls(String) + "E_TLS" to crates/core/src/error.rs (enum + code() + message() arms) and, in request.rs::transport_error, check root_cause for `invalid peer certificate` before the catch-all CoreError::Network at :141, producing 'api.internal presented a certificate this machine does not trust'. The [network]/[tls] config block is deliberately a later item; it is not needed to unblock the common case.

Files: `/Users/d.rus/Projects/mandalo/crates/core/Cargo.toml`, `/Users/d.rus/Projects/mandalo/crates/core/src/request.rs`, `/Users/d.rus/Projects/mandalo/crates/core/src/error.rs`

### 3. Fix the first-run wall: ⌘N must work on an empty workspace

*hours · app*

**Why.** First launch auto-creates ~/Mandalo with an empty tree, both empty states advertise ⌘N, and every route to it produces a red 'Create a collection before adding requests' notice rendered nowhere near the control the user pressed.

**Approach.** In src/store/collection.ts addRequest (:329-335), when slug is undefined and a workspace exists, await the existing createCollection("Scratch") — an explicit user action creating a real folder on disk, announced with a toast 'Created collection Scratch', not a silent fallback — then continue with the returned slug; keep fail() only when workspace is null. Replace the App.tsx:242-246 centre empty state ('Choose one from the sidebar' + kbd hints) with two real buttons: primary 'New request' calling addRequest("http"), secondary 'Import from Postman or OpenAPI' calling useTransfer.openImport, kbd hints below — the import path is currently behind the icon-only TransferMenu ⋯ trigger (TransferMenu.tsx:18-19) that nothing points at. Do not seed a starter request pointing at a public echo endpoint; offline-first means the box ships nothing that assumes a network.

Files: `/Users/d.rus/Projects/mandalo/src/store/collection.ts`, `/Users/d.rus/Projects/mandalo/src/App.tsx`, `/Users/d.rus/Projects/mandalo/src/components/Sidebar.tsx`

### 4. The four first-minute papercuts, including the Description field that permanently breaks autosave

*hours · app*

**Why.** One of these silently stops a request saving forever, two teach a file layout the app no longer writes, and one makes the app's own documented shortcut do something else.

**Approach.** (a) Workbench.tsx:203-205 renders an editable Description that toSaved sends on every autosave (collection.ts:162) and the engine rejects for every text-backed request (reject_description_edit at http_format.rs:1376, grpc_format.rs:570, stream_format.rs:993) — type one character and that request stops saving every 500 ms until you delete it. Make the field read-only on any already-saved text-backed request with the engine's own reason inline: 'the description lives in the # comments above the request — edit the file'. (b) Workbench.tsx:219 says `Saved as requests/{id}.toml`; TOML requests are not merely stale, they are rejected by the loader (collection.rs:809-814). Render the real path, `collections/{draft.collection}/{draft.path}`, plus a Reveal in Finder button via tauri-plugin-opener (already a dependency at src-tauri/Cargo.toml:26 and permitted in capabilities/default.json, with zero uses in src/). (c) Sidebar.tsx:147 'Collections are plain folders of TOML files' → 'A collection is a folder in your workspace. Requests are plain .http files — diffable in git, readable in any editor.' (d) NewMenu.tsx:110 advertises ⌘⇧N on Collection while App.tsx:135-137 binds shift+N to addRequest("graphql"); lift the sidebar's collection prompt into the ui store so App.tsx can trigger it, bind ⌘⇧N to it, make App.tsx:137 addRequest("http") unconditionally, and add a test asserting every NewMenu hint has a matching binding in the App keymap.

Files: `/Users/d.rus/Projects/mandalo/src/components/Workbench.tsx`, `/Users/d.rus/Projects/mandalo/src/components/Sidebar.tsx`, `/Users/d.rus/Projects/mandalo/src/components/NewMenu.tsx`, `/Users/d.rus/Projects/mandalo/src/App.tsx`, `/Users/d.rus/Projects/mandalo/src/lib/collection.ts`

### 5. Make the desktop admit it is git: hygiene on open, repo chip in the header

*days · app*

**Why.** A workspace created in the app is not a repository and has no managed .gitignore, so the secret-leak guard the product already built never runs for anyone who does not independently discover the CLI.

**Approach.** Call git::ensure_git_hygiene(&dir) at the end of the create_workspace and open_workspace handlers in src-tauri/src/lib.rs, ignoring failure when the directory is not a repo — it is idempotent and writes only inside its `# >>> mandalo — managed block` markers (git.rs:5-6), covering *.local.toml, .mandalo/, secrets.toml, .env* (git.rs:9-17). Do it in the Tauri handler, not workspace.rs::scaffold, so core's create path stays free of a git dependency and the CLI's ordering is untouched. Add two-line api.ts wrappers for git_status (lib.rs:684) and git_init (:689) mirroring the ensureGitHygiene wrapper at api.ts:570, and render a chip next to WorkspaceSwitcher: 'Not tracked by git — Initialise' calling git_init when git_status reports no repo, otherwise `branch · N changed`. Extract the findings list already in ExportDialog.tsx into a FindingList component and reuse it from a Workspace health row driven by scan_workspace, with an Install pre-commit hook button over install_precommit_hook (the button press is the consent). Stop there — plan_sync/run_sync is a separate project.

Files: `/Users/d.rus/Projects/mandalo/src-tauri/src/lib.rs`, `/Users/d.rus/Projects/mandalo/src/lib/api.ts`, `/Users/d.rus/Projects/mandalo/src/components/WorkspaceSwitcher.tsx`, `/Users/d.rus/Projects/mandalo/src/store/workspace.ts`, `/Users/d.rus/Projects/mandalo/src/components/ExportDialog.tsx`

### 6. Carry error codes across the IPC edge and give errors a fix button

*days · app*

**Why.** CoreError has 25 deliberately stable codes and edge() throws every one of them away, so a GUI user gets a bare <pre> — including three messages that tell them to open a terminal and run a CLI command for something the app already has a button for.

**Approach.** Change `type Reply<T> = Result<T, String>` (src-tauri/src/lib.rs:11) to Result<T, ApiError> with #[derive(Serialize)] struct ApiError { code: String, message: String } built inside edge() (:15-17) from e.code() and redact::scrub(&e.to_string()) — one function changes, no command signature moves. Add errorCode(e: unknown): string | null beside errorMessage in src/lib/api.ts. Then add a resolution rail to the phase === "error" branch of ResponsePane.tsx:364-373 (currently a chip and a <pre>, nothing else) keyed on code: E_SECRET → 'Set dev.token…' opening EnvModal's existing secret prompt pre-filled (EnvModal.tsx:77-81 already renders exactly that state); E_SECRET_HOST_DENIED → 'Bind dev.token to api.example.com' reusing the handler already present for the unbound case; E_UNRESOLVED_VAR → 'Define {{baseUrl}} in <env>'; default → Retry (re-dispatch useSession.send with the same draft) and Copy error. Separately, delete the ``run `mandalo env set-secret`'' clause from runner.rs::missing_value (:593-600), capability.rs:91 and workspace.rs:486 and have the CLI reporter append it, so core messages are surface-neutral. ApiError + errorCode + Retry is the shippable first cut; do not block on the full rail.

Files: `/Users/d.rus/Projects/mandalo/src-tauri/src/lib.rs`, `/Users/d.rus/Projects/mandalo/src/lib/api.ts`, `/Users/d.rus/Projects/mandalo/src/components/ResponsePane.tsx`, `/Users/d.rus/Projects/mandalo/crates/core/src/runner.rs`, `/Users/d.rus/Projects/mandalo/crates/core/src/capability.rs`

### 7. cURL in and out

*days · core*

**Why.** A curl line is how a request actually arrives — DevTools, docs, Slack — and with no cloud share link the clipboard is the only sharing channel Mándalo has, currently empty in both directions.

**Approach.** New crates/core/src/curl.rs: parse(&str) -> CoreResult<SavedRequest> and render(&SavedRequest, mode) -> String. Parse the flags Chrome actually emits — -X/--request, -H/--header, -d/--data/--data-raw/--data-urlencode, --data-binary @file, -F/--form, -u/--user → basic, --url, bare URL, -G, --compressed, backslash line continuations, POSIX quoting — failing loud and naming any unrecognised flag rather than dropping it. Expose import_curl/export_curl in src-tauri/src/lib.rs and route curl in the CLI's import alongside the existing content sniffing so CLI and GUI agree. Surfaces in value order: (1) onPaste in src/components/UrlBar.tsx — text starting with `curl ` calls import_curl and patches the whole draft with an undoable toast, which is how most curl traffic arrives; (2) "curl" as a fourth ImportKind in src/lib/importKind.ts:1,15 with detectImportKind matching a first non-blank line starting with curl (ImportDialog's Paste tab already exists, only the placeholder at :272 changes); (3) 'Copy as cURL' on the request ⋯ menu and beside Send, rendering secrets as {{var}} placeholders by default and routing any resolve-variables mode through crates/core/src/redact.rs so a copy is safe to paste into a ticket. Snapshot-test render() against the bytes request.rs actually sends so the two cannot drift. Return a loud error for gRPC and stream kinds rather than emitting something that cannot work.

Files: `/Users/d.rus/Projects/mandalo/crates/core/src/curl.rs`, `/Users/d.rus/Projects/mandalo/src-tauri/src/lib.rs`, `/Users/d.rus/Projects/mandalo/crates/cli/src/main.rs`, `/Users/d.rus/Projects/mandalo/src/lib/importKind.ts`, `/Users/d.rus/Projects/mandalo/src/components/UrlBar.tsx`, `/Users/d.rus/Projects/mandalo/src/components/CollectionTree.tsx`

### 8. Duplicate and move a request

*days · app*

**Why.** 'Copy this working request and change one thing' is the most frequent gesture there is and has no implementation, while move_request is fully built, registered and wrapped in api.ts:682 with zero callers — so a misfiled request can only be fixed in Finder.

**Approach.** Duplicate at the byte level, Rust-side: duplicate_request in crates/core/src/collection.rs copying the source block's bytes and appending it with a fresh id and '<name> copy'. Doing it on bytes rather than through the draft model is the point — it preserves comment-borne descriptions and everything else the draft cannot represent (see the Description papercut). Wire it to a Duplicate MenuItem in CollectionTree.tsx's request ⋯ menu, the TabStrip context menu and ⌘D, opening the copy immediately. Move: a 'Move to…' MenuItem opening a flat collection/folder picker calling the already-wired moveRequest, then refreshTree() and retarget open tabs by id. HTML5 drag-and-drop on .tree-row can reuse the same call afterwards — but verify intra-window row dragging works in the webview first, since DropTarget.tsx:15-21 documents that the desktop webview swallows HTML drag events for file drops and routes them through Rust. Reordering is out of scope; it needs an on-disk ordering key.

Files: `/Users/d.rus/Projects/mandalo/crates/core/src/collection.rs`, `/Users/d.rus/Projects/mandalo/src/components/CollectionTree.tsx`, `/Users/d.rus/Projects/mandalo/src/components/TabStrip.tsx`, `/Users/d.rus/Projects/mandalo/src/store/collection.ts`

### 9. Make environments and variables reachable: create, and add-on-the-spot

*days · app*

**Why.** {{baseUrl}} is the first thing anyone types, and today creating the environment to hold it requires finding an unlabelled eye icon → Edit → + New environment, while the app's own precise 'not defined in environment dev' diagnosis is attached to no control at all.

**Approach.** Three edits, all reusing the existing EnvModal and `managing` state, no backend work. (a) In EnvBar.tsx: when envs.length === 0 render a labelled 'New environment' button in place of the select (:97-110); when envs exist, add a gear button (aria-label 'Manage environments') beside it; change the QuickLook empty branch (:42-48) from the sentence 'Select an environment to see its variables' to a button 'No environments yet — create one' → close(); onEdit(). (b) Add an optional { env, key } prop to EnvModal that creates the row on mount and focuses it. (c) In UrlBar.tsx's UnresolvedWarning (:41-66) and the missing branch of VarTokens.tsx (:83-93), render an 'Add {{name}}' button opening EnvModal pre-filled; when no environment exists or none is selected, the button reads 'Create environment and add {{name}}' and calls the env store's create action with the name 'dev' first. Scope to shared environment vars only — wiring the unused set_local_var command (src-tauri/src/lib.rs:225) and a scope picker is separate work.

Files: `/Users/d.rus/Projects/mandalo/src/components/EnvBar.tsx`, `/Users/d.rus/Projects/mandalo/src/components/EnvModal.tsx`, `/Users/d.rus/Projects/mandalo/src/components/UrlBar.tsx`, `/Users/d.rus/Projects/mandalo/src/components/VarTokens.tsx`, `/Users/d.rus/Projects/mandalo/src/store/env.ts`

### 10. Bind the URL bar and the Params table to one value

*days · app*

**Why.** Pasting a URL with a query string — the universal way a request starts — leaves the Params tab empty, so the per-row enable checkboxes apply to nothing and toggling `&debug=true` means editing a single-line string full of {{vars}} and percent-encoding.

**Approach.** Put the binding in the draft layer, which already owns both halves: splitParams runs exactly once at load (src/lib/collection.ts:341, defined :249) and mergeParams only at send/save (src/lib/spec.ts:37, called from collection.ts:160 and spec.ts:67). Lift splitParams into src/lib/spec.ts next to mergeParams and add reconcileParams(url, rows) that re-splits the URL and merges into existing rows, preserving `enabled` and description for rows whose key and value are unchanged and appending the rest. Call it from the store's patch path in src/store/collection.ts on any url change, and call mergeParams on any params change to rewrite the URL's query segment. Keep the raw text authoritative while the URL input holds focus and reconcile on blur so the caret is never fought. The Params tab already shows an active-row count (Workbench.tsx:94) so a pasted URL visibly lands somewhere; spec.test.ts already covers mergeParams, extend it to the round trip.

Files: `/Users/d.rus/Projects/mandalo/src/lib/collection.ts`, `/Users/d.rus/Projects/mandalo/src/lib/spec.ts`, `/Users/d.rus/Projects/mandalo/src/store/collection.ts`, `/Users/d.rus/Projects/mandalo/src/components/UrlBar.tsx`

### 11. Model inherited auth end to end, and clone a workspace from the app

*days · app*

**Why.** Two independent unblocks with the same shape — a concept the engine already ships that the desktop cannot express: inherited auth (currently only guarded, not supported) and joining an existing team workspace, which today means installing the CLI before you can open your team's collection.

**Approach.** (a) Add `| { type: "inherited"; auth: Auth }` to the TS union in src/lib/api.ts:14-18 and src/lib/format/model.ts:10-14, map it both ways in src/lib/collection.ts (removing the guard flag added in item 1), and add an 'Inherit from collection' option to AuthEditor.tsx rendering the wrapped auth read-only with an explicit 'Override for this request' button that unwraps it. Extend the readPreamble allowlist in src/lib/format/httpFormat.ts:249 to @auth and @reconnect so the TS parser can read a file the Rust writer produced (http_format.rs:291 is the reference list), with a parse→render round-trip test. Nothing changes Rust-side; the variant already round-trips. (b) Add a third MenuItem 'Clone from git…' to WorkspaceSwitcher.tsx:107-111 opening a CloneDialog with a URL field and a destination picker, calling a new cloneWorkspace wrapper over git_clone (src-tauri/src/lib.rs:694-697, which already falls back to the stored GitHub token via git_auth at :668-672), then the register/activate path doOpen already uses (WorkspaceSwitcher.tsx:36). Surface CoreError::Private (error.rs:30) with its message; SSH remotes work immediately, device-flow sign-in comes with the sync panel.

Files: `/Users/d.rus/Projects/mandalo/src/lib/api.ts`, `/Users/d.rus/Projects/mandalo/src/lib/format/model.ts`, `/Users/d.rus/Projects/mandalo/src/lib/format/httpFormat.ts`, `/Users/d.rus/Projects/mandalo/src/components/AuthEditor.tsx`, `/Users/d.rus/Projects/mandalo/src/components/WorkspaceSwitcher.tsx`, `/Users/d.rus/Projects/mandalo/src/store/workspace.ts`

### 12. Capture a value and assert on it without writing JavaScript, and a per-request timeout

*days · app*

**Why.** Chaining a token into the next request is the whole reason to have an API client and today it requires hand-writing pm.* against a JSONPath you have to guess; and any endpoint that legitimately takes 45 seconds is permanently unusable because 30s is a constant nobody can raise.

**Approach.** (a) Generate the script rather than invent a second mechanism — the declarative TestAssertion/Capture path is a deliberate dead end (see notDoing). In src/components/JsonView.tsx make each row's value clickable with 'Capture as variable…': the renderer already knows the row's JSONPath, which is the whole reason this is cheap; ask for a name and scope, append `pm.environment.set("token", pm.response.json().access_token)` to draft.testScript, toast with Undo. In ResponsePane.tsx:82 replace 'No tests ran. Write pm.test(…)' with two buttons — 'Assert status is 200' and 'Assert this field exists' — appending the equivalent pm.test block seeded from the response on screen. Rename the Workbench tab key `tests` to `post` and its label from 'Post-response Script' to 'Tests & Script' (Workbench.tsx:20-22,40,94,187) so the tab holding tests is not named after something else. Nothing touches the file format. (b) Add timeoutMs to the request model in crates/core/src/collection.rs, parse `# @timeout 60s` in http_format.rs and its twin src/lib/format/httpFormat.ts, add default_timeout to mandalo.toml, and change request.rs::client() (:152-160) from a single OnceLock to a small HashMap<u64, Client> cache keyed by timeout seconds — keep the map rather than building per send, since reqwest pools connections per client. Update the message at :123 to name the setting that raises it, and expose the field in the Workbench Settings tab.

Files: `/Users/d.rus/Projects/mandalo/src/components/JsonView.tsx`, `/Users/d.rus/Projects/mandalo/src/components/ResponsePane.tsx`, `/Users/d.rus/Projects/mandalo/src/components/Workbench.tsx`, `/Users/d.rus/Projects/mandalo/crates/core/src/request.rs`, `/Users/d.rus/Projects/mandalo/crates/core/src/http_format.rs`, `/Users/d.rus/Projects/mandalo/src/lib/format/httpFormat.ts`

## Later

### Sync panel: plan → review → commit/push, in the app

src/store/git.ts holding SyncStatus refreshed on focus and after saves; a header chip in App.tsx reading `branch · ↑N ↓M · K changed`; src/components/git/SyncPanel.tsx rendering plan_sync's FileChange rows with per-file checkboxes (SyncSelection already models --only/--except), a message box, and Sync / Share as a branch buttons over run_sync and run_branch_push. run_branch_push returns a github_compare_url (git_sync.rs:190) that @tauri-apps/plugin-opener can open — the plugin is already a dependency and already permitted in src-tauri/capabilities/default.json, with zero current uses. Phase 3 is the GitHub device-flow modal over github_start_login/github_poll_login, needed only for private HTTPS remotes.

**The hard part.** The plan→approve→execute UX is the safety property, not decoration: it is what stops a GUI user pushing a token. Conflicted outcomes (SyncOutcome::Conflicted) need a real surface, and mapping FileChange paths back to request names requires the locateRequests map in src/store/collection.ts. This is a product design job, not wiring, which is exactly why the hygiene chip should ship first and separately.

### Collection/folder runner in the desktop

Register run_suite(workspace, collection, folder, env, failFast) in src-tauri/src/lib.rs over the existing Runner::run_suite_with (crates/core/src/runner.rs:420, already called by crates/cli/src/main.rs:757), streaming each StepResult over a tauri Channel using stream_open (lib.rs:774-786) as the working pattern. Add Run collection / Run folder to CollectionTree's ⋯ menus and a RunnerPanel in the response-pane region reusing ResponsePane's test-result rows and the JSON report shape in crates/cli/src/report.rs. Print the equivalent `mandalo run …` invocation at the top of the report so every GUI run is reproducible in CI by copy-paste.

**The hard part.** Streaming progress, cancellation and a re-run-failed path are all new UI state, and the report shape has to stay identical to the CLI's or the two surfaces will drift. Worth doing only after the capture/assert on-ramp exists — running a suite that has no assertions in it produces a green table that proves nothing.

### Cookie jar

New crates/core/src/cookies.rs over the cookie_store crate, fed per redirect hop rather than via reqwest's `cookies` feature — redirects are hand-rolled one hop at a time precisely so HostPolicy re-runs (request.rs:145-158, Policy::none()), and a client-level jar would sit outside that loop. Persist per environment under <workspace>/.mandalo/cookies/<env>.toml and add it to the managed ignore list in crates/core/src/git.rs:86. Then implement pm.cookies.get/has/set/clear and postman.getResponseCookie in crates/core/src/script.rs (replacing the `unavailable` entries at :910 and :960), add cookie.<name> to Capture::from in assertions.rs (the negative test at :748 flips), and surface a Cookies list in the EnvBar popover plus a Cookies tab in ResponsePane.

**The hard part.** Cookies are session state that must never enter the repo, and the jar has to respect HostPolicy on every hop — a cookie attached to a host the policy would refuse is a credential leak. Session-based and CSRF-cookie APIs are a large share of internal web APIs, so this unblocks a whole class of user, but it is a week of careful work, not a feature flag.

### Network configuration: proxy, custom CA, mTLS

An optional [network] table in mandalo.toml (crates/core/src/workspace.rs, Manifest at :31-39) carrying proxy, no_proxy, ca_bundle, client_cert/client_key — committed and reviewable, with passphrases going through the existing secrets.rs store. request.rs's OnceLock client becomes a small cache keyed by a hash of the network config. Order of value: proxy first, then CA bundle, then mTLS. Deliberately no 'skip TLS verification' switch — a named CA bundle is the fail-loud form of the same need, and a committed insecure-hosts list is a footgun a teammate inherits by pulling.

**The hard part.** Behind a corporate proxy the app is unusable rather than degraded, so this is the difference between 'cannot adopt' and 'can adopt' at enterprises — but it is also the first config surface that has to be both committed and secret-aware, and getting that boundary wrong is how a private key lands in git. Ship the native-roots one-liner from the 'now' list first; it removes most of the pressure.

### Saved responses: file export, then examples

Two independent pieces. Export: extend write_text_file_for_export (src-tauri/src/lib.rs:513, wrapped at api.ts:745, zero callers today) with a bytes variant, wire a native save dialog to a Save button in the ResponsePane toolbar, filename defaulted from request name plus the extension implied by Content-Type — this is a day's work and it is the only exit a binary response has. Examples: saved responses live beside the request in a `_examples/` sibling directory, `<request>.<example>.{json,txt,bin}` plus a small .toml sidecar with status, headers and the request snapshot; nothing in http_format.rs changes because the example lives beside the request, not inside it.

**The hard part.** Examples are a new on-disk concept, and every on-disk concept is a format commitment. Stop at 'examples exist and are visible in the tree' — diffing, body-matches-example assertions and a local mock server are downstream ideas that should not be scoped until people actually use examples.

### Source view for the file the request actually is

read_request_source(workspace, collection, path) -> String beside the existing collection::load_request_source (crates/core/src/collection.rs:1064), and a Form | Source segmented control in the Workbench pane header rendering it in the CodeMirror already used by BodyEditor/ScriptEditor. Read-only first: this alone answers 'what am I about to commit' and 'why did the parser reject this'. Editable later, parsing with the TypeScript parser that already ships (src/lib/format/httpFormat.ts) for instant feedback and writing through the span-preserving save_request for the authoritative save.

**The hard part.** The editable half is the real work: two parsers must agree, and today they do not — httpFormat.ts:250 rejects every directive but @name while http_format.rs:291 allows @name, @auth and @reconnect, so the TS parser cannot read back a file the Rust writer produced. Fixing that allowlist and adding a parse→render round-trip test is a prerequisite for anything beyond read-only.

### Runner iterations and data files

run_suite gains an iteration loop; a new IterationData reads a CSV (headers as keys) or a JSON array of objects into Vec<BTreeMap<String,String>>, each row layered into VarFrame above environment vars and below script-set vars; RunReport groups StepResults per iteration. --iterations N and --data <path> on the CLI's Run (crates/cli/src/main.rs:53), one JUnit testcase per assertion per iteration in report.rs. Implement pm.iterationData.get/toObject in script.rs (replacing the unavailable entry at :912) and mirror in src/lib/script/lint.ts, then delete the degradation warning at postman.rs:31-32 and import the collection's data-file reference instead.

**The hard part.** This is the one Postman feature that suits Mándalo better than Postman — the data file is a CSV committed next to the collection, diffed in review, run identically by CI — but it needs the VarFrame layering to be exactly right, and every Postman collection built around the Collection Runner currently arrives structurally degraded, so the importer change is part of the deliverable, not a follow-up.

### Keyboard: a real keymap and a focusable tree

⌘⌥←/→ prev/next tab, ⌘1..9 tab-by-index with ⌘9 = last, ⌘⇧T reopen-last-closed (a bounded closedIds stack in store/tabs.ts, which already owns openIds/dirtyIds), ⌘E env editor, ⌘B toggle sidebar (useLayout.toggle at store/layout.ts:70 exists and is reachable only by a button). Then roving tabIndex over CollectionTree's flattened visible rows — ↑/↓ move, →/← expand/collapse, Enter open, F2 rename, ⌫ delete; every handler already exists and is bound only to onClick. Pair with a command registry (src/lib/commands.ts: Command { id, title, section, hint?, when?, run() }) so CommandPalette stops being a request-jumper and each binding is discoverable.

**The hard part.** CollectionTree is role="tree" with role="treeitem" rows and `grep -c tabIndex` returns 0 — an ARIA tree that cannot be focused is an accessibility defect as well as a power-user dead end, and fixing it means owning focus management across an async-refreshing tree. The registry is the bigger design call: it is the seam that makes every future action keyboard-reachable by default, so it is worth doing deliberately rather than bolting six commands onto the palette.

### VS Code parity: register the built editor, teach it streams

src/lib/vscode/RequestEditor.tsx is a complete webview request editor with tests, compiled by the build:vscode vite mode (package.json:10), and the extension source never references it — no customEditors contribution exists. Register it with priority: "option" for .http so a file opens as text by default and as a form on demand, the same Form/Source duality proposed for the desktop. Separately, contribute a mandalo-stream language for .ws/.mqtt/.sse reusing the existing language-configuration.json, give it the CodeLens treatment .http already has (editor-extension/src/codelens.ts), and add a mandalo.listen command shelling to `mandalo listen <slug> <path>`.

**The hard part.** Dead built code is a smell worth resolving either way — register it or delete it, but not leave it. Streams being invisible in the editor means a third of the protocol matrix has no editor surface at all despite full engine and CLI support, so the grammar work is small but the parity question ('do all three surfaces speak the same vocabulary?') is a standing design decision, not a ticket.

### CLI ergonomics: completions, ad-hoc send, --var

`mandalo completions <bash|zsh|fish|powershell>` via clap_complete off the existing derive tree (a dozen lines), mentioned in init's output. Give send the URL form listen already established — `mandalo send https://api/x -X POST --header 'k: v' --body @file` — reusing listen's flag vocabulary verbatim so the two subcommands stop contradicting each other (main.rs:73-83 vs :86-90). A repeatable --var key=value plus --var-file on run/send/listen, layered above the environment at the top of the resolution chain in crates/core/src/capability.rs, same fail-loud rules, unparseable arguments erroring rather than being ignored.

**The hard part.** Today only secrets can be injected from the environment (MANDALO_SECRET__<ENV>__<KEY>, capability.rs:148), so a CI matrix across staging/preview URLs has to generate or rewrite environment files — --var is the fix, but adding a layer to the variable resolution chain touches every fail-loud path and needs its precedence written down before it is coded.

### Better failure reporting for unreadable and mid-merge files

Change Tree.skipped from Vec<String> to Vec<SkipReason { path, reason, code }> (crates/core/src/collection.rs:213,249 and the push sites at :645,:789,:810), mirror the four `skipped: string[]` declarations in src/lib/api.ts, stop joining in src/store/collection.ts:159-162, and render an expandable notice in Sidebar.tsx with per-file rows and a Reveal action. Separately, teach text_format.rs and its TS twin to recognise a line beginning with `<<<<<<< ` and return CoreError::Conflict (the variant and E_CONFLICT already exist at error.rs:10,48) so a file mid-rebase reads as 'in the middle of a merge' rather than 'unreadable'.

**The hard part.** Low blast radius day to day — it bites after a pull that went wrong — but it is exactly the moment a git-native product has to be better than a cloud one, and today it is worse: three mangled files become one clipped semicolon-soup sentence whose full text is only a hover tooltip.

## Deliberately not building

Postman features Mándalo should stop feeling behind about.

- **Cloud workspaces, accounts, team sync and share links** — Offline-first, no account, nothing phones home. Git is the sync; every collaboration feature must reduce to git operations on the workspace files. Building a second sync path would make the first one optional, and the first one is the product.
- **A 30-language code-snippet sidebar** — curl is the lingua franca and carries ~90% of the value; fetch and httpie are cheap follow-ons over the same render() function if anyone asks. Twenty-eight more targets is cargo cult — surface area to maintain, snapshot-test and keep in sync with what request.rs actually sends, for handoffs nobody makes.
- **A 'skip TLS verification' / insecure-hosts toggle** — Fail loud. A named CA bundle covers every legitimate case (corporate MITM, private CA, self-signed staging) and does so in a form a reviewer can see. A committed insecure-hosts list is a footgun a teammate inherits silently by pulling the repo — the opposite of what git-native is supposed to buy.
- **A declarative test/assertion model in the file format (@test / @capture directives)** — Already settled and deliberately refused: text_format.rs:311-320 and src/lib/format/render.ts:22-31 both point at `> {% … %}` response scripts as the intended home, and no importer emits TestAssertion/Capture. Reopening it is weeks of format work to build a second mechanism for something that already has one. The real gap is a no-code way to *generate* the script, which is in the 'now' list.
- **A built-in mock server** — Nothing phones home and nothing listens. A mock server is a long-running network service with its own lifecycle, ports and state — a different product living inside this one. If saved examples prove popular, serving them is a separate tool that reads the same committed files.
- **API documentation publishing / a hosted docs portal** — Requires an account and a cloud. The git-native equivalent already exists for free: the collection is readable .http text in the repo, reviewed in pull requests, rendered by any git host. Generating a doc site from it is a downstream static-site job, not something the client should own.
- **Real-time collaborative editing and in-app comments on requests** — Comments on a change belong in the pull request, and merge semantics belong to git. A CRDT layer over files that git also edits is two conflicting sources of truth, and it would break the promise that the file on disk is authoritative.
- **Postman's monitors / scheduled cloud runs** — It needs a server that runs your collection while you sleep — an account, hosted execution and stored secrets. `mandalo run` in the user's own CI does the same job with the user's own secrets and their existing alerting, and the data-file runner in 'later' is what makes that genuinely good.
- **Multi-select and bulk operations in the sidebar** — Not a principle, just sequencing: duplicate and move-to are the daily gestures and they do not exist yet. Ship those, then wait for someone to actually ask for bulk. Reordering is separately parked because it needs an on-disk ordering key, which is a format decision that should not ride along on a UI convenience.
- **An in-app diff / merge-conflict resolution editor** — Resolving a merge is what the user's editor and git tooling are for, and every developer who chose a git-native API client already has one. Mándalo's job is to say clearly 'this file is mid-merge' (in 'later') and to get out of the way — not to reimplement a three-way merge UI it will do worse.
