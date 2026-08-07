# The `.http` format

Mándalo stores HTTP and GraphQL requests in `.http` files (`.rest` works too).
One file holds any number of requests, separated by `###`. This page is the
whole format: what it accepts, what it refuses and with what message, and what
it cannot say at all.

Only requests moved to plain text. Environments and the `mandalo.toml` /
`collection.toml` manifests are still TOML, and are documented in
[The workspace](workspace.md).

## Why

A request is text you read in a diff. The old TOML file was a serialisation of
an in-memory struct: it round-tripped perfectly and told you nothing at a
glance. `.http` is the format REST Client and the JetBrains editors already use,
so the file is portable, and a review comment can point at a line that means
something.

The trade is real. Some things a struct can hold — a declarative assertion list,
a disabled row — have no line to live on. Those are listed at the bottom. One
place Mándalo writes something the other clients do not read is
[form-data bodies](#form-data-bodies); the cost is stated there.

## A file

```http
@baseVar = value

### Request name
POST {{baseUrl}}/auth/login
Content-Type: application/json

{
  "user": "ada"
}

> {%
pm.environment.set("token", pm.response.json().token);
%}

### Another
GET {{baseUrl}}/me
Authorization: Bearer {{token}}
```

## The subset

| Element | Rule |
| --- | --- |
| `### text` | Separates requests. The text after `###` is the request name. |
| `# @name text` | The name, when the separator carries none. |
| `# @auth inherited` | Marks the `Authorization` line below it as a collection-wide default the request never asked for. See [Auth](#auth). |
| `METHOD url` | The request line. The method may be omitted and defaults to `GET`. A trailing `HTTP/1.1` (or `1.0`, `2`, `2.0`, `3`) is recognised and is not part of the URL; it stays in the file. |
| indented continuation | A long URL continues on the following indented lines; they are joined with no separator. |
| `Name: value` | A header. Header lines run until the first blank line. `{{vars}}` are allowed in both name and value. |
| body | Everything after the blank line, verbatim, until the next `###`, a `> {%` script, or the end of the file. |
| `< ./path.json` | As the whole body, sends that file. |
| `name = value`, `name = < ./path` | One form field per line, under `Content-Type: multipart/form-data`. See [Form-data bodies](#form-data-bodies). |
| `@name = value` | A file-scoped variable. |
| `< {% … %}` | A pre-request script, written before the request line. |
| `> {% … %}` | A post-response script, written after the body. |
| `# …`, `// …` | Comments, anywhere outside a body. |

Methods: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, `TRACE`,
`CONNECT`.

The body is taken verbatim, so a line inside it beginning with `#` or `//` is
body text, not a comment. A line inside a body beginning with `>` is not: it
ends the body and is read as a script. That is the one place the format is
sharper than it looks.

## Request identity

A request is addressed as `<file>#<index>`, counting from zero:

```sh
mandalo send mock http.http#2
```

The index is canonical — it is what `mandalo ls` prints and what the editor
stores. A name resolves too, when exactly one request in the file carries it:

```sh
mandalo send mock "http.http#Teapot"
```

Two requests sharing a name is not an error in the file; it only makes that name
unusable as an address:

> `2 requests are named "Teapot" — address this one by index instead`

## Variables

`{{name}}` templates are resolved in two passes. The file's own `@vars` go
first, then the environment fills in whatever is left. A file-scoped variable
therefore wins over an environment variable of the same name: it is the more
local of the two, and it sits a few lines above the request that uses it.

```http
@trace = mandalo

### GET echo
GET {{baseUrl}}/get
X-Trace: {{trace}}
```

`@vars` resolve in declaration order, each against the ones before it, so
`@base = https://{{host}}/v1` works when `@host` is declared above it. A name may
hold letters, digits, `_`, `-` and `.`.

Secrets are never `@vars`: a `.http` file is meant to be committed. They are
declared in an environment file and stored outside the workspace — see the last
table on this page.

## GraphQL

A GraphQL request is a normal request marked with one header:

```http
### GraphQL user
POST {{baseUrl}}/graphql
X-REQUEST-TYPE: GraphQL

query User($id: ID!) {
  user(id: $id) { id name }
}

{"id": "u-1"}
```

The body is the GraphQL document; a blank line followed by a `{` starts the
variables object. With no such block the request sends no variables. The marker
header is consumed when the request is read and never reaches the server.

Any other value on that header is refused rather than sent:

> `line 4: X-REQUEST-TYPE only marks a GraphQL request; "grpc" means nothing to Mándalo`

## Auth

Two headers read back as typed auth, because they carry exactly what a typed
auth block carries. Everything else stays a literal header, which puts the same
bytes on the wire.

| Written | Read as |
| --- | --- |
| `Authorization: Bearer {{token}}` | Bearer auth |
| `Authorization: Basic ada:lovelace` | Basic auth — the value is base64-encoded at send time, exactly as REST Client does |
| `x-api-key: mock-api-key` | A header. That is what an API key is. |
| `…/path?api_key=…` | A query parameter. Same. |

A basic-auth username cannot contain a colon, because the colon is the
separator. The file cannot *write* such a username, so the message appears where
one could arrive from somewhere else — saving a request from the app, or an
import:

> `a basic-auth username cannot contain a colon in a .http file: "ada:b"`

### Inherited auth

A Postman import writes the collection's default auth into every request that
did not ask for one, and marks it:

```http
### Profile
# @auth inherited
GET {{baseUrl}}/me
Authorization: Bearer {{authToken}}
```

The marker changes one thing: **an inherited header whose `{{variables}}` are
not set yet is dropped instead of failing the send.** A collection-wide
`Bearer {{authToken}}` is inherited by the login request too — the one whose job
is to produce `authToken` — and a default nobody asked for must not make that
request unsendable. A header the request writes itself carries no marker and
still fails loud with `unresolved variable: authToken`.

Delete the marker line to make the header the request's own. Any other value is
refused:

> ``line 2: `@auth` only takes `inherited` — write the auth itself as an Authorization header``

## File bodies

`< ./path/to/file.json` as the **whole** body sends that file's bytes. The
`Content-Type` comes from the extension unless you set the header yourself.

```http
### Send a JSON file as the body
POST {{baseUrl}}/body/binary

< ./files/payload.json
```

The path is relative to the directory holding `mandalo.toml` — the workspace
root, not the file's own directory. A collection is shared, untrusted
configuration, so a path that could reach outside the workspace is a hard stop
rather than a convenience.

| Written | Message |
| --- | --- |
| `< /etc/passwd` | ``line 4: the body file must be a workspace-relative path, not "/etc/passwd"`` |
| `< C:/keys.pem` | ``line 4: the body file must be a workspace-relative path, not "C:/keys.pem"`` |
| `< ../../secret.json` | ``line 4: the body file must stay inside the workspace: "../../secret.json"`` |
| `<` with no path | ``line 4: the body file needs a path`` |
| `< ./a.json` with more text after it | ``line 4: a `< file` body must be the whole body — no text may follow it`` |
| `<@ ./a.json` | ``line 4: Mándalo does not support `<@` file bodies — use `<` and keep the variables in the request`` |

## Form-data bodies

`Content-Type: multipart/form-data` on the request makes the body a list of
form fields, one per line. Every line is `name = value`; `<` in front of the
value means **read the value from this file**, which is what `< ./file` already
means as a whole body:

```http
### Upload
POST {{baseUrl}}/upload
Content-Type: multipart/form-data

title = Q3 expenses
attachments = < ./files/alpha.txt < ./files/beta.txt
report = < ./files/report.pdf; type=application/x-invoice
```

| Line | Means |
| --- | --- |
| `title = Q3 expenses` | A text field. The value is everything after the first `=`, trimmed. |
| `report = < ./files/report.pdf` | A file field: the value is that file's bytes. The path is workspace-relative, and the part's `Content-Type` comes from the extension. |
| several `< ./path` on one line | Several files under one field name — what a browser posts for `<input type="file" multiple>`. They arrive as separate parts, in file order, sharing the name. In the workbench they show as one key with a file list. Repeating the field name on later lines still reads and folds into the same field. |
| `; type=…` | Sets that field's content type instead of sniffing it from the extension. Written once at the end of the line. `; type=text/plain; charset=utf-8` is one value, not two parameters. |
| `{{vars}}` | Interpolate in names, values and paths, as everywhere else. |

The body block begins after the blank line, exactly like any other body. Blank
lines between fields are allowed, and the spaces around `=` and after `<` are
optional — `photo=<./files/a.png` is the same field as `photo = < ./files/a.png`.
A path is normalised the way a `< ./file` body is, so `./files/alpha.txt` is
stored and rewritten as `files/alpha.txt`.

`<` opens a file reference only when a space or a `.` follows it, so a value that
merely starts with a bracket stays text: `bio = <b>bold</b>` is the string
`<b>bold</b>`. A text value that *would* read back as a path is refused when
written rather than corrupted in silence:

> ``the form field "note" has a value starting with `<`, which a .http file would read back as a file reference``

Mándalo wrote `name < ./path`, without the `=`, for one release. That spelling
still parses to exactly the same field; it is rewritten to `name = < ./path` the
next time the block is saved.

The boundary is **not** written in the file: Mándalo's HTTP client mints a fresh
one for every send, so a boundary in the file would be decoration that never
reaches the wire. Writing one is refused rather than ignored:

> ``line 5: this form body is written as `name = value` lines, which carry no boundary — remove the `boundary=` parameter, because the one on the wire is chosen when the request is sent``

| Written | Message |
| --- | --- |
| `just words` | ``line 5: a form field reads `name = value`, or `name = < ./path` to send a file, not "just words"`` |
| `= orphan` | ``line 5: this form field has no name before its `=` `` |
| `p = < a.png; charset=utf-8` | ``line 5: a form file takes only `; type=…`, not "charset" — every other part header belongs on the request`` |
| `p = < a.png;` | ``line 5: a form file takes one parameter, written `; type=text/plain` `` |
| `leak = < ../../secret.env` | ``line 5: the form-data file must stay inside the workspace: "../../secret.env"`` |
| `leak = < /etc/passwd` | ``line 5: the form-data file must be a workspace-relative path, not "/etc/passwd"`` |

### The boundary form still reads

A file that spells the body out on the wire — the form REST Client, httpYac and
the JetBrains client write — parses to exactly the same request and sends
exactly the same bytes:

```http
### Upload
POST {{baseUrl}}/upload
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name="title"

Q3 expenses
--WebAppBoundary
Content-Disposition: form-data; name="attachments"; filename="alpha.txt"

< ./files/alpha.txt
--WebAppBoundary--
```

Which form a body is written in is decided by the body itself, not by a flag or
a setting: **a body whose first non-blank line begins with `--` is the boundary
form**, and then the `Content-Type` must declare that boundary. Anything else is
the field-per-line form. Nothing is lost by migrating a file in: the boundary
form keeps working, and a block already written that way keeps it when Mándalo
edits it in place, so an import is never reformatted behind your back.

The boundary form is the stricter of the two. A file part references its file
with `< path` — inline bytes are refused — and a text part carries no
`Content-Type`, because that belongs on a file part.

### What this costs

**A form-data request Mándalo writes will not run in REST Client or in the
JetBrains HTTP client.** They only understand the boundary form: to them
`attachments = < ./files/alpha.txt` is a line of body text, and the request goes
out as a text body with no boundary at all. This is the one place where a file
Mándalo writes is not portable, and it is deliberate — the boundary form is
unreadable, and the whole point of a text format is that a person can read it in
a diff.

The symptom is a `400` from the server naming the boundary — `Invalid boundary
for multipart/form-data` or similar. If you see it from Mándalo itself, the
client reading the file is older than the file: update it.

Everything else in the file still works in those clients — the request line, the
headers, JSON and text bodies, `< ./file` bodies, `@vars`, `###` separators,
`# @name`. Only a form-data block is Mándalo-only. If you need one request to run
in both, write that block in the boundary form: Mándalo reads it, sends it
identically, and leaves it alone.

## Scripts

`< {% … %}` before the request line runs before the request is sent. `> {% … %}`
after the body runs when the response arrives. Both run in the same QuickJS
`pm.*` engine Postman collections use, documented in full in
[postman-compatibility.md](postman-compatibility.md).

```http
### 1 Login
< {%
pm.environment.set("username", "ada");
pm.environment.set("password", "lovelace");
%}
POST {{baseUrl}}/auth/login
Content-Type: application/json

{"username": "{{username}}", "password": "{{password}}"}

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
pm.environment.set("token", pm.response.json().token);
%}
```

This is how you assert and how you chain: `pm.test` for assertions,
`pm.environment.set` for captures. There is no declarative alternative.

Only inline scripts run. A file reference stops:

> ``line 9: Mándalo runs inline `> {% … %}` scripts only, not a script file reference``

A response script ends the request, so nothing may follow it:

> ``line 12: nothing may follow a `> {% … %}` response script inside a request``

## What is rejected

Every parse failure names the line. There are no Mándalo-specific directives
beyond `@name` and `@auth`, and an unknown one stops rather than being ignored.

| Written | Message |
| --- | --- |
| `# @timeout 30` | ``line 2: Mándalo does not support the `@timeout` directive — a .http file it writes carries no directives beyond `@name` and `@auth` `` |
| `# @name` with nothing after it | ``line 2: `@name` needs a name`` |
| `@my var = x` | ``line 1: "my var" is not a valid variable name`` |
| `FETCH {{baseUrl}}/x` | ``line 3: "FETCH" is not an HTTP method — Mándalo supports GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, TRACE, CONNECT`` |
| a request line with no URL, `GET` on its own included | ``line 3: this request line has no URL`` |
| a `###` block with no request line | ``line 3: this block has no request line — a request needs `METHOD url` `` |
| a header line with no colon | ``line 4: expected `Name: value` or a blank line before the body, found "Accept json"`` |
| `Bad Header: x` | ``line 4: "Bad Header" is not a valid header name`` |
| a `> {%` that is never closed | ``line 9: this script block is never closed with `%}` `` |
| an index past the end | ``this file holds 12 requests, so there is no request 40`` |
| an unknown name | ``no request named "Login" in this file`` |

A `###` separator with no name and nothing under it is not an error — it is how
people close a file. A named separator that declares no request is.

## What the format cannot express

These are not gaps to route around; they are things a `.http` file has no line
for. Saving a request that needs one fails with the message shown, so nothing is
dropped in silence.

| Not expressible | What happens | Where it lives instead |
| --- | --- | --- |
| A form field disabled rather than deleted | ``a .http file cannot keep a disabled form field — enable it or remove it`` | Delete the line, or comment the whole request out. |
| A form field whose value spans lines or is padded with spaces | ``the form field "notes" has a value spanning more than one line, which a .http file cannot write — keep it on one line`` | One line per field is the format. Write the boundary form by hand if a value truly needs newlines. |
| A structured urlencoded body | ``a .http file writes a form body as text — set Content-Type: application/x-www-form-urlencoded and write `a=1&b=2` as the body`` | A text body, which sends identical bytes. |
| Declarative `[[tests]]` and `[[captures]]` | ``a .http file cannot carry declarative tests or captures — assert and capture from a `> {% … %}` response script instead`` | A `> {% … %}` script. |
| A gRPC call | ``a gRPC request belongs in a .grpc file, not a .http file`` | [The `.grpc` format](grpc-format.md). |
| A websocket or an MQTT connection | ``a websocket stream belongs in its own file, not a .http file`` | [`.ws` and `.mqtt`](stream-formats.md). Server-sent events *do* live here: a request that sends `Accept: text/event-stream` is a stream, and needs no file type of its own. |
| An API key placed anywhere but a header or the URL | ``a .http file writes an api key as a header or a query parameter, not as "cookie" — put it in the URL instead`` | A header line, or a query parameter in the URL. |
| A per-request description **as an editable field** | ``a .http file keeps a description in the `#` comments above the request, so it cannot be edited as a field — change the comment in the file`` | A `#` comment above the request. Creating a request *with* a description writes those comment lines for you; the parser then reports the description as `None`, because a comment is prose and guessing which one was "the description" would duplicate it on every save. |
| A secret value and the hosts it may be sent to | Nothing to fail on — a `.http` file never held one. | An environment file: `token = { secret = true, hosts = ["api.example.com"] }`. The value never enters the workspace; it is read from `MANDALO_SECRET__<ENV>__<KEY>`. |
