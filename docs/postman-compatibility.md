# Postman compatibility

Mándalo imports Postman v2.1 collections and environments, and runs the `pm.*`
scripts they ship with in an embedded QuickJS sandbox. This page is the honest
inventory: what runs unchanged, what imports with a caveat, and what is not
supported — including the exact message you will see.

Everything below is measured by `crates/core/tests/postman_compat.rs`, which
runs 95 real Postman idioms plus two hand-written collections
(`crates/core/tests/fixtures/postman/`) on every build. The test fails if any
idiom fails *silently* or with a bare JavaScript error.

Measured today: **77 idioms run exactly as in Postman, 18 stop with a named
limitation, 0 fail confusingly.**

## What imports perfectly

| Postman feature | Notes |
| --- | --- |
| Folders, nested folders | Become real directories on disk |
| Request order inside a folder | Kept: a folder with more than one request numbers its files `1-login.http`, `2-me.http`, … so `mandalo run` runs them in the order the collection lists them |
| Collection variables | Saved as an environment named `<collection>-vars` |
| Environment exports | Disabled values are dropped |
| `{{variable}}` templates | Identical syntax; passed through untouched |
| URL objects (protocol/host/port/path/query) and raw URLs | Disabled query params dropped |
| Path variables (`:id`) | Substituted from the export's `url.variable` values (`/users/:userId` → `/users/{{userId}}`) |
| Raw bodies | The editor language (`options.raw.language`) is kept, and drives the default `Content-Type` |
| urlencoded bodies | Structured rows, keeping `disabled` and `description`; the `Content-Type` comes from the mode |
| form-data bodies | Imported as the field lines the format writes — `title = Q3 expenses`, one row per field. A text-only body arrives whole and silent. A file field arrives as a row with no file (below), because the export names a path on the author's disk |
| Binary (`file`) bodies | The request imports without the body; the export's absolute path is reported so you can write `< ./your-file` yourself (below) |
| GraphQL bodies | Query and variables |
| Bearer, Basic and API-key auth (header or query) | Including collection- and folder-level defaults and `noauth` opt-outs — see [Inherited auth](#inherited-auth) |
| Request-level pre-request and test scripts | Run for real, at the same points Postman runs them |
| Disabled headers, disabled variables | Dropped |
| A request written as a bare URL string | Imported as a GET |

## What imports with a caveat

**Collection- and folder-level scripts are copied into every request below
them.** Postman keeps one shared script and runs it before/after every request
in scope. Mándalo's file format has no shared scripts, so the import inlines a
copy into each request, prefixed with
`// inlined by the Postman import: folder "Users" pre-request script`.
Behaviour matches Postman on the first run; if you later edit the script you
must edit each copy. The import report names the scope and every request that
received a copy:

> `Users: pre-request script copied into every request below it (List users, Create user, Get user, Delete user) — Mándalo has no shared scripts, so edit each copy`

### Inherited auth

Postman's collection- and folder-level auth is a **default**, and Mándalo keeps
it a default. Three rules, in this order:

1. A request that declares its own `auth` — `noauth` included — never also
   receives the default.
2. A request that already writes its own `Authorization` header keeps it, and
   the default is not added on top. (Two `Authorization` lines were the old
   behaviour; only one was ever sent.) The import says so:

   > `Me: the request writes its own Authorization header, so the collection's default auth was not added on top of it`

3. Every other request gets the default, written as an ordinary header under a
   marker line:

   ```http
   ### Profile
   # @auth inherited
   GET {{baseUrl}}/me
   Authorization: Bearer {{authToken}}
   ```

The marker is what makes a login-then-use collection runnable. A collection
whose auth is `Bearer {{authToken}}` gives that default to the login request
too — the one whose job is to *produce* `authToken`. An inherited header whose
variables are not set yet is therefore **dropped at send time** instead of
failing the request; a header the request wrote itself still fails loud with
`unresolved variable: authToken`. The value is a default nobody asked for, so
its absence is not an error; your own header is a promise, so it is.

**OAuth 2.0 becomes a bearer token.** Mándalo does not run the OAuth flow. If
Postman had stored an access token, it is imported as a bearer token:

> `OAuth2 stored token: OAuth 2.0 imported as the access token Postman had stored — Mándalo does not run the OAuth flow, so refresh the token yourself when it expires`

With no stored token the request is imported unauthenticated:

> `WithoutToken: OAuth 2.0 has no stored access token — imported without auth; paste a token into a bearer variable to send it`

**File fields import unresolved, and say which file they wanted.** The body
itself comes across: every field becomes a
[form-data line](http-format.md#form-data-bodies). What cannot come across is
the file *contents*. A Postman export stores a file part as an absolute path on
the machine that exported it (`"src": "/Users/ada/pic.png"`), while a collection
references files by workspace-relative path — it is shared configuration, and it
must never be able to make the app read a file outside the workspace. So the
field arrives as a row with no file, the path the export named is kept in the
`#` comment above the request, and the report names the request and the field:

```
Upload avatar: file field `avatar` needs a file inside the workspace — the export referenced /Users/ada/pic.png; the field arrived empty, so point it at one with `avatar = < ./your-file`
```

Copy the file into the workspace and write the path on that row:

```http
### Upload avatar
POST {{baseUrl}}/upload
# Form-data file fields with no file yet — point each at one inside the workspace:
# avatar = < ./your-file   (the export referenced /Users/ada/pic.png)
Content-Type: multipart/form-data

caption = hola
avatar = < ./files/pic.png
```

An empty file row still sends — as an empty text part — so the request runs and
the server tells you what it thinks of it; fill the row in to send the file.

A multi-file field (Postman writes `src` as an array) stays **one** row naming
every path it referenced, because one field holding several files is what the
export meant; write the paths under that one name, one line each.

Two more things a `.http` file has no line for, both named in the report rather
than dropped in silence: a **disabled** form field (it writes only the fields it
sends) and a `contentType` on a **text** field (`; type=` belongs to file rows).

A binary (`file`) body gets the same treatment:

> `Raw file body: the binary body needs a file inside the workspace — the export referenced /Users/dev/blob.bin`

**Variable scopes are one store.** `pm.environment`, `pm.globals`,
`pm.variables` and `pm.collectionVariables` read and write the same set of
variables. A value set through one scope is visible through all of them. Nothing
is lost, but a collection that relies on the *same name* meaning different
things in the global and environment scope will not behave as it does in
Postman.

**A path variable with no value stays literal.** If the export has
`:orderId` with an empty value there is nothing to substitute:

> `Empty: path variable :orderId has no value in the export — it stays literal in the URL, set it before sending`

**`pm.info.requestId` is generated per run.** Postman's id comes from its
cloud model; Mándalo generates a fresh UUID for each execution.

## What is not supported

Each of these stops the script at that line with the message shown. They never
fail silently and never surface a bare `ReferenceError`. Scripts that use them
are also flagged **at import time**, per request, e.g.
`Unsupported script apis: script uses pm.sendRequest, which Mándalo does not support — …`.

| Feature | Message |
| --- | --- |
| `pm.sendRequest` | `pm.sendRequest is not available in Mándalo scripts: scripts cannot make network requests — chain a real request in the collection and capture what you need from its response` |
| `pm.execution.*` (`setNextRequest`, `skipRequest`) | `pm.execution is not available in Mándalo scripts: run-flow control (setNextRequest, skipRequest, location) is not implemented — requests run in collection order` |
| `postman.setNextRequest` (legacy) | `postman.setNextRequest is not available in Mándalo scripts: run order is the collection order; branching is not implemented` |
| `pm.iterationData` / newman data files | `pm.iterationData is not available in Mándalo scripts: there is no data-file runner (newman -d); pass the value as an environment variable instead` |
| `pm.cookies` | `pm.cookies is not available in Mándalo scripts: there is no cookie jar; send cookies as an explicit Cookie header` |
| `pm.vault` | `pm.vault is not available in Mándalo scripts: the Postman vault is not implemented; use a Mándalo secret` |
| `pm.visualizer` | `pm.visualizer is not available in Mándalo scripts: response visualizations are not implemented` |
| `pm.response.to.have.jsonSchema(...)` | `pm.response.to.have.jsonSchema is not available in Mándalo scripts: no JSON-schema validator is bundled — assert on the fields you care about with pm.expect` |
| `require("crypto-js")` and any `require()` | `require is not available in Mándalo scripts: there is no module loader; scripts cannot import code` |
| `CryptoJS` | `CryptoJS is not available in Mándalo scripts: crypto-js is not bundled; compute the signature outside the script and pass it in as a variable` |
| `_` (lodash) | `_ is not available in Mándalo scripts: lodash is not bundled; plain JavaScript array and object methods are available` |
| `moment` | `moment is not available in Mándalo scripts: moment is not bundled; use the built-in Date object` |
| `xml2Json` | `xml2Json is not available in Mándalo scripts: the XML helper is not bundled; assert on the raw text with pm.response.text()` |
| `tv4` / `Ajv` | `… is not available in Mándalo scripts: no JSON-schema validator is bundled; assert on the fields you care about with pm.expect` |
| `Buffer` | `Buffer is not available in Mándalo scripts: the node Buffer is not available; use btoa/atob for base64` |
| Async tests (`pm.test(name, function (done) {…})`) | `pm.test's done() callback is not available in Mándalo scripts: tests run synchronously — drop the done parameter and assert inline` |
| `setTimeout` / `setInterval` / `fetch` / `process` / `window` | `<name> is not available in Mándalo scripts: <why>` |
| An assertion Mándalo does not implement | `pm.expect(...).bananas is not available in Mándalo scripts: that chai assertion is not implemented — supported ones are …` |
| A response state Mándalo does not implement | `pm.response.to.be.teapot is not available in Mándalo scripts: supported response states are ok, success, info, redirection, clientError, serverError, error, accepted, badRequest, unauthorized, forbidden, notFound, rateLimited, withBody, json, html, xml` |
| A string method on `pm.request.url` | `pm.request.url.replace is not available in Mándalo scripts: pm.request.url is a Postman URL object, not a string — call pm.request.url.toString() before using string methods` |
| A dynamic variable with no generator | `{{$randomBankAccountName}} is not available in Mándalo scripts: that Postman dynamic variable has no generator here — supported ones are $guid, $randomUUID, …` |

Collection-level features that are not scripts:

| Feature | Behaviour |
| --- | --- |
| Schema v2.0 or v1 collections | Import fails loudly: `unsupported Postman schema: … (only v2.1 collections are supported)` — re-export as v2.1 |
| `awsv4`, `digest`, `hawk`, `ntlm` auth | Imported without auth: `AWS v4: awsv4 auth is not supported — imported without auth, so the request goes out unauthenticated` |
| Saved example responses (`item.response`) | Ignored |
| `protocolProfileBehavior` | Ignored |
| Cookies, proxy and certificate settings | Not imported |

## The script API Mándalo implements

### Assertions

`pm.test(name, fn)`, `pm.expect(value)`, `pm.expect.fail(message)`.

Chai: `equal` / `eq` / `equals`, `eql` / `eqls`, `deep.equal`, `a` / `an`,
`above` / `greaterThan`, `below` / `lessThan`, `least`, `most`, `within`,
`closeTo`, `include` / `contain` / `contains` / `includes`, `string`,
`property(name[, value])`, `keys` (with `all` / `any`), `members`, `lengthOf` /
`length`, `match`, `oneOf`, `satisfy`, and the getters `true`, `false`, `null`,
`undefined`, `ok`, `empty`, `exist`, `finite`, `NaN` — each with a `.not` form,
and the usual `to / be / been / is / that / which / and / has / have / with /
at / of / all / own` filler words.

Response: `pm.response.to.have.status(codeOrText)`,
`.have.header(name[, value])`, `.have.body([textOrRegExp])`,
`.have.jsonBody([pathOrObject[, value]])`, and the states `ok`, `success`,
`info`, `redirection`, `clientError`, `serverError`, `error`, `accepted`,
`badRequest`, `unauthorized`, `forbidden`, `notFound`, `rateLimited`,
`withBody`, `json`, `html`, `xml` — all with `.not`.

### Response

`pm.response.code`, `.status`, `.responseTime`, `.responseSize`, `.text()`,
`.json()`, `.reason()`, `.size()`, `.headers.get/has/all`.

### Request

`pm.request.method`, `.url` (a URL object with `toString`, `query.add/upsert/
remove/get/has/all/count/clear/each/toObject`, `path`, `host`, `protocol`,
`getPath`, `getHost`, `addQueryParams`, `removeQueryParams`), `.body` (with
`.raw`), `.headers.add/upsert/remove/has/get/all/each/count`. Assigning to
`pm.request.url` or `pm.request.body` works and is applied to the request that
is actually sent — but only from a **pre-request** script, which is where
Postman applies it too.

### Variables

`get`, `set`, `unset`, `has`, `clear`, `toObject`, `replaceIn(template)` on
`pm.environment`, `pm.globals`, `pm.variables` and `pm.collectionVariables`
(one shared store — see the caveat above).

### Dynamic variables

Generated both at interpolation time (`{{$guid}}` inside a URL, header or body)
and inside scripts via `pm.variables.replaceIn`:

`$guid`, `$randomUUID`, `$timestamp`, `$isoTimestamp`, `$randomInt`,
`$randomBoolean`, `$randomAlphaNumeric`, `$randomWord`, `$randomWords`,
`$randomFirstName`, `$randomLastName`, `$randomFullName`, `$randomUserName`,
`$randomEmail`, `$randomPassword`, `$randomPhoneNumber`, `$randomCity`,
`$randomCountry`, `$randomStreetAddress`, `$randomCompanyName`,
`$randomJobTitle`, `$randomProductName`, `$randomDomainName`, `$randomUrl`,
`$randomIP`, `$randomColor`, `$randomHexColor`, `$randomDatePast`,
`$randomDateFuture`.

Any other `{{$…}}` fails loudly and lists the supported ones. A variable you
define yourself always wins over the generator of the same name.

### Legacy (pre-`pm`) sandbox

Collections exported years ago still work: `tests["name"] = booleanExpression`
(collected as test results), `responseBody` (with `.has()`), `responseCode`
(`.code`, `.name`, `.detail`), `responseTime`, `responseHeaders`, and
`postman.setEnvironmentVariable` / `getEnvironmentVariable` /
`clearEnvironmentVariable` / `setGlobalVariable` / `getGlobalVariable` /
`clearGlobalVariable` / `getResponseHeader`.

### Environment

`console.log/info/debug/warn/error` (captured into the run log), `btoa`, `atob`,
`pm.info.requestName / requestId / eventName / iteration / iterationCount`, and
the standard JavaScript library (ES2020: `let`/`const`, arrow functions,
template literals, spread, `JSON`, `Math`, `Date`, `RegExp`).

Scripts are sandboxed: no network, no filesystem, no host process, no timers,
32 MB and 2 s per script. Exceeding either is reported as
`script exceeded 2000ms` / `script exceeded the memory limit of 33554432 bytes`.

## Is Mándalo Postman compatible?

For the common case — a REST collection with folders, `{{baseUrl}}`, bearer or
basic auth, a login request that captures a token, and `pm.test` blocks that
assert on status, headers and JSON — yes: it imports and the tests run
unchanged.

It is not compatible with collections built around the runner (data files,
`setNextRequest`), around `pm.sendRequest` warm-ups, or around npm modules
(`crypto-js` request signing). Those import, and tell you exactly where and why
they stop. Multipart uploads are the one body a `.http` file cannot hold: the
request imports without it, and the report says which fields were left behind.

## Exporting back to Postman

`mandalo export out.json --format postman --collection <slug>` writes a v2.1
collection. Setting `[share] format = "postman"` in `mandalo.toml` makes Export
default to that format and makes Sync regenerate `postman/<slug>.json` (plus
environments) before each commit. Native Mandalo files remain the source of
truth; the Postman JSON is a mirror for teammates who stay in Postman. gRPC and
stream requests are skipped with a named warning; declarative `tests[]` /
`captures[]` are Mandalo-only and are likewise named, not silently dropped.
