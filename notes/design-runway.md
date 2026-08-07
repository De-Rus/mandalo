# Runway — a design direction for Mándalo

Produced by a nine-agent design panel: an inventory of the current UI, four
independent redesign visions, three judges (founder / engineer / daily Postman
user), and a synthesis. **This is a proposal, not shipped work.**

## Direction

**Runway — the app draws the flow, and amber only ever means "this moved or this isn't committed yet."**

The redesign is not a repaint; it is shipping the product already sitting on disk. `src/lib/format/model.ts` defines `Capture{from,into,scope}` and `TestAssertion`, `crates/core/src/runner.rs` executes them, `StepResult.captured` comes back on every send — and `grep` finds them in exactly zero components, meaning that today a user must hand-edit TOML to chain a login token. Meanwhile `git_status`, `plan_sync` and `run_sync` are registered Tauri commands that no line of `src/` calls. Runway takes the flow between requests and the state of the working copy — the two things Mándalo already knows and no cloud client can know — and makes them the primary graphics of the app: a request produces values, a Ledger holds them while you watch them mutate, a wire shows which request feeds which, and a 24px git bar shows what of that is still yours alone. Every gesture writes plain TOML your teammate reviews in a PR and the CLI runs unchanged in CI. The interaction/visual language is called **Signal**: the existing near-black IDE surface stays, brand amber `#F6A821` is demoted from generic accent to a single meaning — *live or uncommitted* — and a paler `--signal #FFCE6A` is reserved exclusively for the instant a value moves. A clean checkout at rest is monochrome; when the screen lights up, it is telling you something true.

## The three wow moments

### 1. Click-to-chain: hover a token in the response, press ⇥, watch it fly into your environment

**Looks like** — Hovering any leaf line in `JsonView` raises a 3-button rail flush to the line's right edge — `⎘ path · ⇥ Capture · ✓ Assert` — in mono, 11.5px, `--bg-elevated`, 4px radius, no shadow. Press ⇥ and a one-line inline editor drops in pre-filled: name inferred from the key (`data.access_token` → `accessToken`), scope segmented `run | session | persist` with a guess already selected and one muted sentence of reason under it ("looks like a credential — session, never written to disk"). Enter commits: the request's Chain tab count ticks to 1, the tab dot goes amber (dirty), and a 4px `--signal` bead lifts off the JSON line, arcs over the response head and lands on a Ledger row that materializes with a 320ms background flash.

**Behaves like** — ⇥ appends a real `Capture{from:"$.data.access_token", into:"accessToken", scope:"session"}` to the draft — the same struct the CLI already honours — and marks the request dirty. `✓ Assert` does the same for `tests[]`, defaulting to `json`+`exists` for objects, `equals` for scalars, and always adding `status == 200` if no status assertion exists yet. ⌘⇧C / ⌘⇧A do it from the keyboard at the caret's line. Scope defaults are shown, never silent: a credential-shaped value that the user forces to `persist` gets an inline warning that it will land in a git-tracked file. One bead, one arc, never two in flight; `prefers-reduced-motion` replaces the flight with a 90ms flash on source and destination.

### 2. The Ledger: environment state as an instrument panel that visibly mutates on every send

**Looks like** — A 300px right dock (⌘L, collapsed by default, auto-peeks for 4s when a capture lands). Four columns: name, value in mono with secrets masked, a source chip, and scope. The source chip is the payoff — `env`, `⟵ POST /login`, `script`, `keychain`, `local` — and the request chip is clickable. `run`-scoped rows carry a dotted underline; `session` rows a moon glyph and the tooltip "dies when Mándalo quits, never touches disk"; `persist` rows a `●M environments/dev.toml` git chip in amber whenever that write is not yet committed.

**Behaves like** — During a send the Ledger is the second thing that moves after the send state: when `StepResult.captured` arrives, the affected row's old value slides up 8px with a 40%-opacity strikethrough, the new value slides in beneath, a `--signal` background flash decays over 320ms, and the row hoists into a 'just changed' group for 3 seconds. Clicking a `⟵ POST /login` chip opens that request with its Chain tab focused on the row that produced the value. A footer line reads `run 2 · session 1 · persist 4 · 1 unbound secret`, the last linking to the existing host-binding flow. Provenance is never inferred silently: a name written by `pm.environment.set` in a script shows the `script` chip, not a fake producer edge.

### 3. The Amber Law: a calm monochrome app that blooms when you have uncommitted work, and visibly cools when you commit

**Looks like** — At rest on a clean checkout the app is six greys plus method badges — the active tree row and active tab lose their amber and become `--bg-selected` with a neutral `#6f7783` rail and a 600-weight label. Amber is now scarce and therefore loud. Touch anything and it blooms in three coordinated places: a 2px rail in a new 10px status gutter at the sidebar's left edge (`M` modified, `A` new, `D` deleted), a dot on the tab, and a count in a new 24px bottom **GitBar**: `⑂ main · ↑2 ↓0 · 3 changed · ⚠ 1 secret · [Review ⌘⇧R] · [Copy CI command]`.

**Behaves like** — Reads `git_status` — already a registered Tauri command returning branch, ahead/behind, staged/unstaged/untracked, conflicted and `dirtyFiles` — on mount, on save, and on window focus. `Review ⌘⇧R` opens a sheet built on `plan_sync`, which already returns a per-file changeset plus `scan::Finding` secret findings and a `blocked` flag, and describes the change in API nouns: "POST /login — capture added: accessToken (session)", "environments/dev.toml — 1 value changed (value hidden)". Findings block the commit until moved to the keychain or explicitly acknowledged; secret values are never rendered. On successful commit, a 400ms desaturation sweep runs top→bottom down the sidebar, the amber rails cross-fade out and the GitBar counter tweens 3 → 0. If the workspace is not a repo the bar says `not a repo · [Initialize]` and every git surface is disabled with that reason — never a silent zero.

## App specification

## Shell

Two structural additions to the existing three-pane shell. Header 44px, sidebar 262px, zero-width `Splitter`, `responseRatio` 0.45 all survive; migration risk is a new flex child and a new sibling row.

```
┌ header 44 ──────────────────────────────────────────────────────────────────────┐
│ mA │ acme-api ▾                              [dev ▾] [◉ Ledger ⌘L] [▶ Run ⌘R]  │
├──┬──────────────┬────────────────────────────────────────────┬──────────────────┤
│▏ │ FLOW RAIL 262│ tabstrip 34                                │ LEDGER 300 (⌘L)  │
│▏ │ ⌕ search     ├────────────────────────────────────────────┤──────────────────│
│▏ │              │ [HTTP][POST] {{baseUrl}}/login    [Send ⌘⏎]│ name   value  src│
│▏ │ ▾ Auth       ├────────────────────────────────────────────┤ token  ⟨token⟩ ⟵ │
│M │  ●POST login⟩│ Params Auth Headers Body Chain·1 Assert·2 …│         POST /login
│▏ │   ╎╲         ├────────────────────────────────────────────┤ userId 42    ⟵ me│
│▏ │ ▾ Users      │  panel                                     │ baseUrl https…env│
│▏ │ ⟨○GET  me    │══════════════ splitter ════════════════════│──────────────────│
│▏ │ ⟨○POST user  │ RESPONSE 200 OK  118ms 1.2kB  ▮▮▮▯▮▮ ✓3 ✗0 │ run 2 · session 1│
│▏ │              │ Body Headers⁷ Chain Console                │ persist 4 ●M     │
│  │ ⋯ Billing (3)│ [Tree│Table│Raw]  ⌕     path: data.token ⧉ │ 1 unbound secret │
│  │              │  2   "token": "⟨token⟩"      ⎘  ⇥  ✓        │                  │
│  │              │ ✓3   "id": 42               $.id exists    │                  │
├──┴──────────────┴────────────────────────────────────────────┴──────────────────┤
│ ⑂ main · ↑2 ↓0 · 3 changed · ⚠1 secret   [Review ⌘⇧R] [Copy CI command]         │ 24
└─────────────────────────────────────────────────────────────────────────────────┘
 ▲ 10px status gutter: M/A/D = working-copy state.  ⟩ outlet = produces values.
 ⟨ inlet = consumes a captured value.  ╎╲ = wire (phase 2).
```

## Per-pane changes

**Sidebar → Flow Rail** (`Sidebar.tsx`, `CollectionTree.tsx`). Rows stay 28px/15px indent. Two new 5px gutters: a right-edge **outlet dot** on any request with non-empty `captures[]`, a left-edge **inlet chevron** on any request whose interpolated surface references a captured name. Indentation math (`4 + depth*15`) shifts by one constant for the 10px status gutter; nothing else in the tree changes. Hovering a row dims non-connected rows to 35% over 160ms and lights its up/downstream neighbours. Active row is neutral now, not amber.

**Workbench** (`Workbench.tsx`). `TAB_IDS.http` becomes `params · auth · headers · body · chain · assert · scripts · settings`. **Chain** is the missing captures editor (`from` JSONPath, `into` name, `scope` segmented, plus a live preview column showing what the last response would have produced). **Assert** is the declarative `tests[]` editor (status / json / header / duration rows). Today's two script tabs merge into one **Scripts** tab with a segmented pre/post switch — scripts are the escape hatch, not two of eight top-level nouns. Both new tabs reuse the existing `.count` / `.tab-dot` idiom. Ship tab selection persisted per request at the same time, so the new layout does not feel like the app forgot where you were.

**Response pane** (`ResponsePane.tsx`, `JsonView.tsx`, `lib/json.ts`). Body toolbar becomes **Tree · Table · Raw**. Table mode appears only when a node is an array whose first 20 elements are all objects; under ~60% key overlap it still renders but shows a `sparse: 41% key overlap` notice rather than a grid of dashes. Columns sort, and the pinned-column set persists per request id — `GET /orders` opens forever as exactly the three fields you care about. Headers tab becomes a decoded grid driven by a pure-TS `HEADER_CODECS` table (~40 entries): `cache-control` directives as chips with `max-age` rendered `5 min`, `set-cookie` expanded with a `Secure missing` warning off localhost, `x-ratelimit-*` as an inline meter, `link` `rel=next` as a button that sends page 2 in a new tab; every row keeps a `raw ⌄` disclosure, decoding is additive and never lossy. Declarative assertions render as gutter chips against the exact line they assert — a pass is a quiet 10px `✓` at 70% opacity, a failure gets a red left rail plus an inline `expected 200 · actual 404` row and a `Fix` link into the Assert tab. An unresolvable path is its own state: `$.data.id — path not found in this response`, never a silent false. A 6-tick **filmstrip** at the head's right edge holds the last runs of this request; click to replay one (marked `replayed · 12:41`), shift-click two for a structural diff. **No phase waterfall**: `ResponseData` exposes exactly `durationMs`, so we draw one honest number. Anything more would be an invented segment, and inventing a segment is the same sin as a silent fallback.

**GitBar** (new, 24px, sibling of `.app-body`). Branch, ahead/behind, dirty count, secret-finding count, `Review ⌘⇧R`, `Copy CI command` (emits the exact `mandalo run` invocation for the current collection + env — the cheapest GUI→CI bridge in the product).

## New components

`GitBar.tsx` · `ReviewSheet.tsx` · `Ledger.tsx` + `LedgerRow.tsx` · `ChainEditor.tsx` · `AssertEditor.tsx` · `NodeActions.tsx` (the ⎘/⇥/✓ rail, reusing `ContextMenu`'s portal + viewport clamp) · `CaptureInline.tsx` · `HeadersGrid.tsx` + `lib/headerCodecs.ts` · `TableView.tsx` · `Filmstrip.tsx` · `FlowDots.tsx`, later `FlowWires.tsx` · `store/git.ts`, `store/ledger.ts`, `store/history.ts`.

## Motion rules

Four tiers, all `cubic-bezier(.4,0,.2,1)`: **90ms** hover/press · **160ms** panel, tab, dim-to-focus · **320ms** state mutation (Ledger row swap, signal flash) · **420ms** bead flight, plus the one 400ms commit drain. Nothing else animates. Only data moves — panels never slide, menus never bounce. Never more than one bead in flight. Everything above 160ms is gated on `prefers-reduced-motion`, where beads become a single 90ms flash on source and destination: the information survives, the choreography doesn't. Ship a `Reduce flow animation` setting that keeps the flash and drops the flight, because flying dots read as a toy to exactly the senior backend engineer who buys this.

## tokens.css evolution

Keep every surface and the six greys — this is a deepening, not a repaint. Additions:

- `--brand: #f6a821` stays, but its **job narrows** to identity, the Send button, and the alias `--chg` (uncommitted). It is removed from `.tree-row-active` and `.tab-active`, which become `--bg-selected` + `--rail-neutral: #6f7783` + 600 weight.
- `--signal: #ffce6a` — reserved absolutely for the instant a value moves. Users learn: *pale amber = something just moved.*
- Change trio: `--chg: var(--brand)`, `--add: #4bbd7a`, `--del: #ef5f5f`, `--stg: #5b9dfb` — identical meaning in the status gutter, the review sheet, the response diff and the filmstrip.
- Wire group: `--wire`, `--wire-dim`, `--wire-unknown` (grey, dashed — edges we cannot statically prove), `--wire-glow`.
- Legitimize the off-scale drift: `--fs-code: 12.5px`, `--fs-label: 11.5px`. Mono is promoted from a code affordance to the typeface for all **values** (ledger values, capture paths, chips, durations, git surfaces); UI font keeps the labels. `font-variant-numeric: tabular-nums` locked on everywhere a number can change.
- Radii unchanged (4/6/10). The 999px pill is explicitly rejected, including in the extension webview.
- Depth without blur: a 1px inner top highlight `rgba(255,255,255,.03)` on elevated surfaces; `--shadow-lg` spent only on true overlays. The Ledger and GitBar are flat panels behind 1px `--border-subtle` hairlines, because they are part of the workbench, not floating over it.

## VS Code extension specification

The extension already has `codelens.ts`, `tree.ts`, `testing.ts`, `results.ts` and `media/results.css`, so every move below extends a wired surface rather than inventing one. Goal: the two products teach the *same mental model* — flow and working copy — using the affordances VS Code actually gives us.

1. **One token file, two themes.** Generate `editor-extension/media/tokens.css` from `src/styles/tokens.css` at build time, mapping each Mándalo token to its `--vscode-*` equivalent with the Mándalo value as the *fallback* (`background: var(--vscode-editor-background, var(--bg-app))`). Kill the webview's 999px pills in favour of the desktop's 4px chip and adopt `--add/--del/--chg/--signal` verbatim. One diff, and the two products stop looking like they came from different companies.
2. **A third CodeLens on every `###` block: `⇥ 2 captured · ⟵ 1 consumed`.** Highest leverage change available, because CodeLens is already file-pattern based and gated by an existing setting — extend `src/core/lenses.ts` and nothing else moves. Seeing what a request produces and consumes sitting above a request you are about to debug is free context.
3. **Flow view.** A second view in the existing activity-bar container, `mandalo.flow`, listing **Feeds into** / **Fed by** for the selected request as clickable rows. VS Code will not let us draw beziers; the compensation is that the same information is one keystroke away instead of zero. Request `description` slots also carry it: `POST · ⇥2`, `GET · ⟵token, userId`.
4. **The results webview becomes an authoring surface.** Its Captures tab is a passive table today; give each row `into ← from`, a scope chip, and an **Insert `{{into}}` at cursor** button that posts back and edits the active editor. A new **Assert on last run…** CodeAction on any `.http` request opens a QuickPick of JSONPaths harvested from the most recent run and inserts the `[[tests]]` block — the same closed loop as the desktop node-actions rail, on a native surface.
5. **SCM state in the tree.** Use `vscode.FileDecorationProvider` so request nodes get the same `M`/`A`/`U` badges and `gitDecoration.*` theme colours as the Explorer, rolled up to collection nodes. This is the Amber Law expressed in VS Code's own vocabulary rather than fighting it.
6. **Status bar as a two-item ledger.** Keep `$(globe) dev`; add `$(symbol-variable) 4 vars · 2 captured`, whose click opens a QuickPick of current session state with source labels. Run-scoped values are shown explicitly labelled `from last run — stale`, never silently presented as current.
7. **Test Explorer gains captures and real diffs.** `syncAssertions` already models collection → folder → request → assertion; add a capture node group under each request so the tree reports what a request *produced*. Populate `TestMessage.actualOutput` / `expectedOutput` so VS Code renders its own side-by-side assertion diff instead of today's flat `failureSummary` string — a two-line change with an outsized quality read.
8. **New commands in the Mándalo category:** `Capture from last run…`, `Review Changes` (the desktop review sheet as a webview reusing its markup), and a `mandalo://` deep link so the extension and the desktop app hand work to each other instead of duplicating it.

## Build order

### 1. The Amber Law + GitBar — wire the git commands nobody is calling — _night_

`git_status` is already a registered Tauri command returning branch, ahead/behind, staged/unstaged/untracked, conflicted and `dirtyFiles`, and no line of `src/` calls it. Add the typed `invoke` wrapper, a tiny store that refreshes on mount / save / window focus, and a 24px bottom bar in mono `--fs-label`. In the same pass do the colour reassignment in tokens.css: add `--signal`, `--chg/--add/--del/--stg`, demote amber off `.tree-row-active` and `.tab-active` to `--bg-selected` + neutral rail + 600 weight, and add the 10px status gutter to `.tree-row` as a `::before` column (one constant shift to the `4 + depth*15` indent math). Ship the 400ms commit drain and the counter tween the same night. Not a repo → `not a repo · [Initialize]`, every git surface disabled with that reason. Visible wow on night one: the app is calm until you type, then amber blooms in three places at once and drains when you commit.

Files: `src/lib/api.ts`, `src/store/git.ts`, `src/components/GitBar.tsx`, `src/styles/tokens.css`, `src/styles/app.css`, `src/components/CollectionTree.tsx`, `src/App.tsx`

### 2. Chain and Assert tabs — make the format's own features authorable — _night_

`Capture{from,into,scope}` and `TestAssertion` are in `model.ts`, honoured by the runner and reported by the CLI, and referenced by zero components. Add two tabs to `TAB_IDS.http` over the existing `KeyValueEditor` idiom: Chain (from / into / scope segmented) and Assert (status / json / header / duration rows), with `.count` badges and dirty dots for free. Merge the two script tabs into one Scripts tab with a segmented pre/post switch, and persist tab selection per request while you are in there. Pure frontend over an already-typed model — highest value-per-line change in the repo, and it closes the hole where chaining a token today means hand-editing TOML.

Files: `src/components/Workbench.tsx`, `src/components/ChainEditor.tsx`, `src/components/AssertEditor.tsx`, `src/lib/draft.ts`, `src/lib/collection.ts`, `src/store/collection.ts`

### 3. Click-to-chain from the response — ⎘ path · ⇥ Capture · ✓ Assert — _night_

`tokenizeLines` in `lib/json.ts` is line-based; extend it to emit a JSONPath per line (a depth stack over the tokens it already produces — no tree-model rewrite needed, which is the architectural advantage over a full virtualized explorer). Then a hover rail on each leaf line, portalled through `ContextMenu`'s existing viewport clamp, with an inline capture editor pre-filled from the key and a scope defaulted by value shape: credential-looking → `session` (never written to disk), ids/cursors → `run`, else `persist`, with the reason shown as one muted sentence. Enter appends a real Capture to the draft — which the tab built in step 2 now displays. ⌘⇧C / ⌘⇧A for the keyboard path. Ship the `--signal` flash on commit of the capture now; the bead flight arrives with the Ledger.

Files: `src/lib/json.ts`, `src/components/JsonView.tsx`, `src/components/NodeActions.tsx`, `src/components/CaptureInline.tsx`, `src/lib/vars.ts`, `src/store/collection.ts`

### 4. The Ledger dock — provenance, scope, and the bead landing — _week_

300px right flex child in `.app-body`, ⌘L, collapsed by default with a 4s auto-peek when a capture lands; re-derive `CENTER_MIN` with the dock open so a 13-inch screen stays honest, and keep the existing EnvBar eye popover as the compact path. Merge four sources into one table with a provenance chip (`env`, `⟵ POST /login`, `script`, `keychain`, `local`) — `StepResult.captured` and the env store already carry the data. Add the 320ms row swap, the scope affordances, the `●M environments/dev.toml` chip, `Reset run state`, and now the 420ms bead flight from JSON line to Ledger row with the full reduced-motion path.

Files: `src/components/Ledger.tsx`, `src/components/LedgerRow.tsx`, `src/store/ledger.ts`, `src/store/env.ts`, `src/store/layout.ts`, `src/App.tsx`, `src/styles/app.css`

### 5. Flow dots + inline assertion chips — _week_

Derive the producer→consumer graph from data already in memory: `captures[].into` gives produced names, scanning url/headers/params/body/auth/graphql vars for `{{name}}` gives consumed names. Render as outlet dots and inlet chevrons in the tree gutters plus hover dim-to-focus — the judges are right that this carries ~70% of the wire's value with none of the SVG geometry, so it ships before any bezier. In parallel, render declarative `tests[]` as gutter chips against the asserted line in the response, with the red rail, the inline expected/actual row, and `path not found in this response` as its own explicit state.

Files: `src/components/FlowDots.tsx`, `src/lib/flow.ts`, `src/components/CollectionTree.tsx`, `src/components/JsonView.tsx`, `src/components/ResponsePane.tsx`

### 6. Review sheet over plan_sync — commit as a list of things you did — _week_

`plan_sync` already returns a per-file changeset with `scan::Finding` secret findings and a `blocked` flag, and `github_compare_url` already exists — this is UI over shipped Rust, not new plumbing. Render the changeset in API nouns ("POST /login — capture added: accessToken (session)", "environments/dev.toml — 1 value changed (value hidden)"), each row expandable to the real TOML hunk, each a stage checkbox. Pre-fill the commit message from those same semantics. Findings block the button until moved to the keychain or acknowledged; secret values never render. Footer: Commit · & Push · & open PR.

Files: `src/components/ReviewSheet.tsx`, `src/lib/api.ts`, `src/store/git.ts`, `src/components/GitBar.tsx`

### 7. Decoded headers grid — _week_

`HEADER_CODECS`, ~40 entries of pure TS with no Rust and no new state, turning the two-column string table into typed rows: cache-control directive chips with humanised max-age, set-cookie sub-table with a `Secure missing` warning off localhost, x-ratelimit meters, and `link rel=next` as a button that sends page 2 in a new tab. Every row keeps `raw ⌄` — decoding is additive, never lossy. Cheapest credibility moment available: it is the point where the app stops printing HTTP and starts understanding it.

Files: `src/lib/headerCodecs.ts`, `src/components/HeadersGrid.tsx`, `src/components/ResponsePane.tsx`

### 8. Table mode with per-request pinned columns — _week_

Array-of-objects renders as a sortable grid, union-of-keys headers, sparse cells as `—`, pin set persisted per request id so `GET /orders` always opens as the three fields you care about. Offered only when the first 20 elements are all objects; under ~60% key overlap it renders with an explicit `sparse` notice rather than silently guessing. The single biggest per-day time saving in the whole set of pitches.

Files: `src/components/TableView.tsx`, `src/lib/table.ts`, `src/components/ResponsePane.tsx`, `src/store/layout.ts`

### 9. Filmstrip + structural run diff — _week_

Last N=25 runs per request, in memory by default, optional `.mandalo/history/` that is gitignored and never exported (a captured token must not land on disk by surprise — opt-in only, and stated in ExportDialog's existing 'Deliberately excluded' section). Six ticks in the response head, click to replay, shift-click two for a structural diff reusing the existing tokenizer with added/removed/changed row states. Answers 'it worked this morning' in two clicks.

Files: `src/store/history.ts`, `src/components/Filmstrip.tsx`, `src/lib/diff.ts`, `src/components/ResponsePane.tsx`

### 10. Extension parity pass — _week_

Generated tokens.css with `--vscode-*` fallbacks and the death of the 999px pill; the `⇥ 2 captured · ⟵ 1 consumed` CodeLens; the `mandalo.flow` view; actionable Captures rows with Insert-at-cursor; FileDecorationProvider SCM badges; the two-item status bar; capture nodes and real `actualOutput`/`expectedOutput` in the Test Explorer. Each is independently shippable and each makes the two products teach the same model.

Files: `editor-extension/media/tokens.css`, `editor-extension/media/results.css`, `editor-extension/src/codelens.ts`, `editor-extension/src/tree.ts`, `editor-extension/src/testing.ts`, `editor-extension/src/results.ts`, `editor-extension/src/commands.ts`

### 11. The wires — _month_

Only now, and only because the dots already delivered most of the value: a single absolutely-positioned SVG over the tree scroll container drawing beziers between visible outlet/inlet coordinates. Three edge classes and the distinction is the entire point — solid amber = proven from `captures[]`; dotted grey = resolved from the env file (state, not flow); dashed with a `?` cap = written by `pm.environment.set` inside a script, which we cannot verify without running it, and whose tooltip says exactly that. Fail-loud extends to inference; a confident amber line we cannot prove is a lie and must never be drawn to make a screenshot prettier. Batch geometry in one rAF off the scroll listener, draw only edges with a visible endpoint, cap at ~60 before falling back to dots-only.

Files: `src/components/FlowWires.tsx`, `src/lib/flow.ts`, `src/components/Sidebar.tsx`, `src/styles/app.css`

### 12. Run Mode timeline — _month_

⌘R transitions the centre pane to one 28px lane per request matching the tree's rhythm, a real-time playhead, duration bars tinted by status, test outcomes as staggered 6px squares, and a bead flying from each producing lane to its consuming lane. Scope discipline is absolute: the timeline may draw only what `mandalo run` actually did — sequential, one env, stop-or-continue. No retries, no parallelism, no data-driven iterations implied by pixels the runner cannot back. Ends as a pinned run card with `Copy CI command` and `Copy summary`, which is worth more than the animation.

Files: `src/components/RunTimeline.tsx`, `src/store/run.ts`, `src/lib/api.ts`, `src/App.tsx`

### 13. ⌘E buffer mode — the form IS the file — _month_

The one idea worth stealing wholesale from Runline, saved for last because it is the only item here needing a research-grade invariant: a lossless printer plus a parse→print→parse identity property test across every fixture in the grammar (`render.ts` is 180 lines, `edit.ts` 64 — surgical edit exists, round-trip fidelity does not). Toggle the workbench between panels and a CodeMirror view of the exact bytes on disk, with a git gutter matching the GitBar. Anything the panels cannot losslessly represent disables the toggle with an explicit reason — 'this request has constructs the form cannot represent; edit as text' — and never silently rewrites the user's file. Skip the FLIP morph in v1; a 90ms cross-fade buys 90% of the effect for none of the jank. This is the move that turns 'git-native' from a bullet point into something the user can see.

Files: `src/components/BufferView.tsx`, `src/lib/format/render.ts`, `src/lib/format/httpFormat.ts`, `src/lib/format/roundtrip.test.ts`, `src/components/Workbench.tsx`

