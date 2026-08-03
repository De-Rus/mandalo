# The `.http` format

Mándalo stores HTTP and GraphQL requests in `.http` files (`.rest` works too).
One file holds any number of requests, separated by `###`. This page is the
whole format: what it accepts, what it refuses and with what message, and what
it cannot say at all.

Only requests moved to plain text. Environments and the `mandalo.toml` /
`collection.toml` manifests are still TOML.

## Why

A request is text you read in a diff. The old TOML file was a serialisation of
an in-memory struct: it round-tripped perfectly and told you nothing at a
glance. `.http` is the format REST Client and the JetBrains editors already use,
so the file is portable, and a review comment can point at a line that means
something.

The trade is real. Some things a struct can hold — a multipart body with
per-part content types, a declarative assertion list — have no line to live on.
Those are listed at the bottom.

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
| `METHOD url` | The request line. The method may be omitted and defaults to `GET`. A trailing `HTTP/1.1` (or `1.0`, `2`, `2.0`, `3`) is recognised and is not part of the URL; it stays in the file. |
| indented continuation | A long URL continues on the following indented lines; they are joined with no separator. |
| `Name: value` | A header. Header lines run until the first blank line. `{{vars}}` are allowed in both name and value. |
| body | Everything after the blank line, verbatim, until the next `###`, a `> {%` script, or the end of the file. |
| `< ./path.json` | As the whole body, sends that file. |
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
separator:

> `a basic-auth username cannot contain a colon in a .http file: "ada:b"`

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

## Scripts

`< {% … %}` before the request line runs before the request is sent. `> {% … %}`
after the body runs when the response arrives. Both run in the same QuickJS
`pm.*` engine Postman collections use, documented in full in
[postman-compatibility.md](postman-compatibility.md).

```http
### 1 Login
POST {{baseUrl}}/auth/login
Content-Type: application/json

{"username": "ada", "password": "lovelace"}

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
beyond `@name`, and an unknown one stops rather than being ignored.

| Written | Message |
| --- | --- |
| `# @timeout 30` | ``line 2: Mándalo does not support the `@timeout` directive — a .http file it writes carries no directives beyond `@name` `` |
| `# @name` with nothing after it | ``line 2: `@name` needs a name`` |
| `@my var = x` | ``line 1: "my var" is not a valid variable name`` |
| `FETCH {{baseUrl}}/x` | ``line 3: "FETCH" is not an HTTP method — Mándalo supports GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, TRACE, CONNECT`` |
| a request line with no URL | ``line 3: this request line has no URL`` |
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
| A multipart form-data body with per-part content types | ``a .http file cannot express a multipart form-data body with per-part content types`` | Nowhere. A request that needs one has to stay in a client that models parts. |
| A structured urlencoded body | ``a .http file writes a form body as text — set Content-Type: application/x-www-form-urlencoded and write `a=1&b=2` as the body`` | A text body, which sends identical bytes. |
| Declarative `[[tests]]` and `[[captures]]` | ``a .http file cannot carry declarative tests or captures — assert and capture from a `> {% … %}` response script instead`` | A `> {% … %}` script. |
| A gRPC call | ``a gRPC request belongs in a .grpc file, not a .http file`` | [The `.grpc` format](grpc-format.md). |
| An API key placed anywhere but a header or the URL | ``a .http file writes an api key as a header or a query parameter, not as "cookie" — put it in the URL instead`` | A header line, or a query parameter in the URL. |
| A per-request description **as an editable field** | ``a .http file keeps a description in the `#` comments above the request, so it cannot be edited as a field — change the comment in the file`` | A `#` comment above the request. Creating a request *with* a description writes those comment lines for you; the parser then reports the description as `None`, because a comment is prose and guessing which one was "the description" would duplicate it on every save. |
| A secret value and the hosts it may be sent to | Nothing to fail on — a `.http` file never held one. | An environment file: `token = { secret = true, hosts = ["api.example.com"] }`. The value never enters the workspace; it is read from `MANDALO_SECRET__<ENV>__<KEY>`. |
