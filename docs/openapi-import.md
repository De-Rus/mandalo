# OpenAPI / Swagger import

Mándalo turns an API specification into a runnable collection of `.http` files.
This page is the honest inventory: what is imported, what is imported with a
caveat, what is refused, and the exact message you will see.

Everything below is measured by `crates/core/tests/openapi.rs`, which imports
seven hand-written specs (`crates/core/tests/fixtures/openapi/`) and then **runs**
the result against the testkit mock on every build.

```bash
mandalo import ./openapi.yaml
mandalo import https://api.acme.com/openapi.json
```

A url is fetched by Mándalo itself, never by the webview: it passes the same
host policy as any request you send (so a cloud-metadata address is refused),
every redirect hop is checked again, and the transfer stops at 64 MB. The
document has to be text — a body that is not valid UTF-8 is refused rather than
imported mangled, and a 404 is reported as an error instead of imported as if
the error page were a specification.

The importer is picked from the file's content, not its extension: a document
with a top-level `openapi` or `swagger` field goes to this importer, a Mándalo
bundle to the bundle importer, anything else to the Postman one.

## Versions

| Version | Status |
| --- | --- |
| OpenAPI 3.1 | Imported |
| OpenAPI 3.0 | Imported |
| Swagger 2.0 | Imported — converted on the way in (below) |
| Swagger 1.x, OpenAPI 4.x | Refused by name: `unsupported Swagger version: 1.2 — Mándalo imports Swagger 2.0, OpenAPI 3.0 and OpenAPI 3.1; convert this document first` |
| No version field | Refused: `this file is not an API specification: it has no top-level `openapi` (3.0/3.1) or `swagger` (2.0) version field` |

**JSON and YAML** both import. YAML is parsed with
[`serde_norway`](https://crates.io/crates/serde_norway) — the maintained fork of
`serde_yaml`, which its author archived in 2024.

Swagger 2.0 is converted mechanically: `schemes` × `host` + `basePath` become
servers, `consumes` becomes the request content type, an `in: body` parameter
becomes the request body, `in: formData` parameters become a form body,
`definitions` resolve exactly like `components/schemas` (both are plain
`#/…` pointers), and `securityDefinitions` map onto the same auth table as 3.x —
`type: basic` is read as `http`/`basic`.

## Security: nothing is fetched, ever

**A `$ref` that points outside the document is refused, not resolved.** Importing
a file makes no network call and reads no other file:

> `this specification references another document: $ref "./shared/owner.yaml#/Owner", "https://schemas.example.com/pet.json#/Pet". Mándalo never fetches a reference while importing — bundle the spec into a single self-contained file (redocly bundle, swagger-cli bundle) and import that`

The check runs over the whole document *before* anything is written, so a spec
that is refused leaves the workspace exactly as it was. Only `#/…` pointers into
the same document resolve, `~0`/`~1` escapes included.

**YAML anchors are refused.** A chain of anchors is how a one-kilobyte file
expands into gigabytes, and the expansion happens inside the YAML parser where
no budget can see it. No OpenAPI generator emits them:

> `this YAML uses a node anchor (&shared) — Mándalo does not expand YAML anchors, because a chain of them turns a small file into an unbounded one; run the spec through a bundler that resolves them and import the result`

**Everything else is bounded.** A hostile spec cannot hang or exhaust the app:

| Bound | Limit | What happens past it |
| --- | --- | --- |
| Document size | 8 MiB | Refused before it is parsed |
| Parsed nodes | 400 000 | Refused after parsing, before anything is written |
| Requests written | 2 000 | Imported up to the limit; the rest is one `skipped` line naming the operation it stopped at |
| Named request examples per operation | 10 | The first ten become requests; the rest are one `skipped` line naming the example keys |
| Response example in a comment | 14 lines / 700 chars | Cut, with `… cut here — the whole example is in the specification` |
| `$ref` hops in one chain | 32 | Refused: `it points at itself` |
| Generated example depth | 8 levels | The branch ends in `null` and the report says why |
| Generated example nodes | 2 000 | Same |
| Recursive schema | — | Cut the first time a type appears on its own path |

## What is imported

| Spec | Becomes |
| --- | --- |
| `paths` × method | One request per operation |
| `operationId`, else `summary`, else `METHOD /path` | The request name |
| `summary` + `description` | The `#` comment block above the request |
| `tags` | Folders; an untagged operation sits at the collection root |
| `servers` | One environment per server, each with `baseUrl` |
| Server variables | Environment variables, seeded from `default` (or the first `enum` value) |
| `deprecated: true` | `# DEPRECATED in the specification.` in the comment block |
| Path parameters | `{{variables}}` (below) |
| Required query/header parameters | Real query parameters and headers, as `{{variables}}` |
| Optional parameters | Listed in the comment block (below) |
| Cookie parameters | Folded into one `Cookie` header |
| `requestBody` | A body in the matching content type (below) |
| Named `requestBody` examples | One request per example (below) |
| Named parameter examples | The first seeds the variable; the rest go in the comment block |
| The first success response's example | Quoted in the comment block as reference (below) |
| `security` / `securitySchemes` | Auth (below) |

### Path parameters become variables

`/pets/{petId}` is imported as `{{baseUrl}}/pets/{{petId}}`, and `petId` is added
to every environment, seeded from the parameter's `example`, then `default`, then
the first `enum` value, then a value shaped by its `type` and `format` (a `uuid`
parameter is seeded with a uuid-shaped string, a `date-time` with a timestamp).

This is a deliberate trade. Keeping OpenAPI's own `{petId}` would be literal and
useless — you would have to edit every URL by hand. `{{petId}}` is a real
variable you set once in the environment and every request that takes a pet id
follows. The cost is that the seeded value is a placeholder, not your data:
**check the environment before you send anything that writes.**

Required *query* and *header* parameters are treated the same way, so
`?limit={{limit}}` and `X-Tenant: {{X-Tenant}}` are set in one place. A parameter
whose name is not a legal variable name (anything outside letters, digits, `-`,
`_`, `.`) is written as its literal seed instead. A path that templates a
parameter the operation never declares stays literal, and the report says so.

### Optional parameters are documented, not disabled

A `.http` file has no disabled row — a header is either a line or it is not
there. So an optional parameter is written into the comment block above the
request, where a reader will find it, with the spec's own description:

```
### listPets
# Returns a page of pets.
# Optional parameters the spec declares — a .http file has no disabled row, so add the ones you want:
#   status={{status}} — Filter by status
#   X-Request-Id={{X-Request-Id}}
GET {{baseUrl}}/pets?limit={{limit}}
```

Move the line where it belongs — into the query string, or down among the
headers — and it is sent. Nothing is lost; it just is not armed.

### Bodies

The body comes from the spec's `example`, then the first entry of `examples`,
and otherwise **is generated from the schema** — which is the part that makes an
import usable rather than a list of empty POSTs. The generator handles `$ref`,
`allOf` (every arm contributes, later arms win), `oneOf`/`anyOf` (the first arm,
reported), `enum` (first value), `const`, `default`, arrays (one element), and
3.0's `nullable` as well as 3.1's `type: [string, "null"]` — the non-null arm is
the useful one. `format` drives the scalar: `date-time` → `2026-01-01T00:00:00Z`,
`date` → `2026-01-01`, `uuid` → a uuid-shaped string, `email`, `uri`, `ipv4`,
`byte` and the rest likewise; a plain string is `"string"`, an integer `0`, a
boolean `true`.

Properties are written in alphabetical order, not the spec's order — the
importer's JSON model sorts keys, and a stable order beats a coincidental one.

When an operation offers several content types, one is chosen — JSON first, then
form-encoded, then XML, then text, and multipart last — and the report names the
others. A `multipart/form-data` body becomes
[form-data field lines](#multipartform-data-bodies-become-field-lines); any
other multipart subtype is skipped by name. A binary body
(`application/octet-stream`, `image/*`, …) is imported without a body and the
report tells you to write `< ./your-file` under the request.

### `multipart/form-data` bodies become field lines

A multipart body is generated from its schema exactly like the JSON example
body: one [form-data line](http-format.md#form-data-bodies) per property, seeded
from `example` → `default` → the first `enum` → a shape from `type`/`format`.

A property that carries bytes — `format: binary` in 3.0, `contentMediaType` in
3.1, or an array of either — is a **file** field. Nothing in a spec names a file
a workspace could hold, so that row imports with no file, the comment block says
what the spec declared, and the report says so per operation:

```
uploadPetPhoto: the form-data file fields (file) arrived empty — a spec names no file a workspace could hold, so point each at one with `file = < ./your-file`
```

```http
### uploadPetPhoto
POST {{baseUrl}}/pets/{{petId}}/photo
# Form-data file fields with no file yet — point each at one inside the workspace:
# file = < ./your-file   (the spec declares it as a file, sent as image/png)
Content-Type: multipart/form-data

caption = string
file =
```

An empty file row still sends — as an empty text part — so the request runs and
the server tells you what it thinks of it; fill the row in to send the file.

`encoding.<property>.contentType` is what a part is sent as: on a file row it is
kept in that comment and becomes `; type=…` once you write the path; on a text
row it is dropped and named in the report.

An all-text multipart body imports whole, with no warning at all, and sends the
parts the schema described.

Swagger 2.0 says the same thing differently: `in: formData` parameters with one
`type: file` among them are imported as this same body, and the report says the
body was written as multipart/form-data because no other body carries a file.

### Named examples become one request each

A spec author who wrote three named examples wrote them for a reason. When
`requestBody.content.<type>.examples` names several, the operation is imported
**once per example**, named `operation (key)`, with the example's `summary` in
the comment block:

```
### createPet (full)
# Add a pet
#
# Body: the spec's "full" example — Everything the API accepts
POST {{baseUrl}}/pets
Content-Type: application/json

{ "breed": "corgi", "name": "Fido", "tags": ["good", "boy"] }
```

Nothing changes for an ordinary spec. A bare `example` (the singular shorthand,
which wins over `examples` when both are present) and a *single* named example
each produce one request under the operation's own name — the single named one
still says which example it is in the comment, but it gains no suffix. The report
names the fan-out when there is one:

> `createPet: the spec names 3 request examples — one request was imported for each (full, minimal, rescue)`

Examples are ordered by key, so the files land in a stable order. Past ten on one
operation the rest are dropped and named:

> `postCase: 4 of the 14 named request examples were not imported — an operation writes at most 10 requests (case10, case11, case12, case13)`

### Parameter examples: one seeds, the rest are offered

A parameter can carry `examples` too, but an environment holds **one** value per
variable. The first named example seeds it; the others go into the comment block
where they can be read and pasted:

```
### getPet
# Other parameter values the spec gives as examples — a variable holds one at a time:
#   petId=pet_404 — example "missing": A pet it does not, to see the 404
GET {{baseUrl}}/pets/{{petId}}
```

Up to five alternatives per parameter are listed. They are **not** imported as
anything runnable, and this page would rather say so than let you assume they
were.

### Response examples are quoted, never asserted

The example the spec gives for the first success response (`200` before `201`,
`default` if there is no `2xx`) is quoted in the comment block, clipped to 14
lines, so you can see what a good response looks like without leaving the file:

```
### getPet
# Example 200 response from the specification — reference only, nothing here asserts it:
# {
#   "id": "pet_1",
#   "name": "Fido"
# }
```

**No assertion is generated from it, and none from the status code either.** A
test written from an example body fails the moment a real server returns one
extra field, and a suite that is red on the first run teaches people to delete
tests rather than read them. The status code has the same problem here for a
different reason: an import seeds path and query parameters with *placeholders*,
so `GET /pets/{{petId}}` with a made-up id legitimately returns 404 — a generated
`pm.test("status is 200")` would fail on a correct server. The imported suite is
green on the first run and stays honest about what it checked: nothing. Write the
assertions you want in a `> {% … %}` block; the next import will not overwrite
them, because it writes to a new collection.

A recursive schema terminates. `Node.parent: Node` produces:

```json
{ "id": "string", "label": "string", "parent": null, "children": [null] }
```

and a warning: `createNode: the example body stops early because the schema Node
contains itself — it is a skeleton, not a complete request`.

### Auth

`security` on an operation overrides the document-wide `security`; `security: []`
on an operation means **explicitly none** and nothing is inherited.

| Scheme | Becomes |
| --- | --- |
| `http` / `bearer` | `Authorization: Bearer {{SchemeName}}` |
| `http` / `basic` | Basic auth with `{{SchemeNameUser}}` / `{{SchemeNamePassword}}` |
| `apiKey` in `header` | The header itself, e.g. `X-Api-Key: {{SchemeName}}` |
| `apiKey` in `query` | Appended to the URL: `?api_key={{SchemeName}}` |
| `apiKey` in `cookie` | `Cookie: name={{SchemeName}}` |
| `oauth2` | Imported **unauthenticated**, with the scheme and flow named |
| `openIdConnect` | Imported unauthenticated, with the scheme and discovery URL named |
| Anything else (`mutualTLS`, HTTP `digest`, …) | Imported unauthenticated, named |

> `readProfile: OAuth 2.0 ("OAuth2", the authorizationCode flow) imported without auth — Mándalo does not run OAuth flows; paste a token into a bearer variable to send it`

The token variables are **declared, never invented**: each one is written into
the environment as a *secret* with no value, so `mandalo env set` (or the app's
environment editor) is where you put it. Nothing that looks like a credential is
ever written into a committed file.

Bearer and basic auth are written as an **inherited** default (`# @auth
inherited` above the request). A security requirement is something the document
declares, not something you wrote into the request, so an unset token drops the
header instead of stopping the run — the rest of the suite still executes and the
authenticated calls come back 401. Set the variable and they authenticate. The
report says this once:

> `auth is declared but has no value here: set BearerAuth with `mandalo env set` — until then those requests go out unauthenticated`

An api key in a header cannot be marked inherited — a `.http` file writes it as
the header it always was, and reads it back as a header — so an unset api key
**stops** that request on the unresolved variable rather than sending without it.
The report names it per operation.

Two schemes required at once (`{A: [], B: []}`) or several alternative
requirements: the first is imported and the rest are named.

## What is not imported

| Spec feature | Behaviour |
| --- | --- |
| `webhooks` (3.1) | Not imported — a webhook is a request the API sends *to you*, so there is nothing here to send. Reported by count. |
| `callbacks`, `links` | Ignored |
| Response schemas, and every response but the first success one | Ignored. The first success response's *example* is quoted in the comment block; nothing else about a response is imported, and nothing is asserted. |
| Operation-level `servers` | Ignored; the document's servers win |
| `parameters` with `style` / `explode` / `deepObject` | The name and a value are imported; the serialisation style is not modelled |
| `discriminator` | Ignored — `oneOf` takes its first arm |
| Named request examples past the tenth | Dropped, named in the report |
| Parameter examples past the fifth | Dropped from the comment block |
| A named example's `externalValue` | Ignored — reading it would be a fetch |
| `xml`, `readOnly` / `writeOnly` | Ignored |
| `encoding` outside `multipart/form-data` | Ignored. Inside it, `contentType` names what a file part is sent as; on a text field it is named in the report and dropped, because `; type=` belongs to a file row |
| Vendor extensions (`x-…`) | Ignored |

### What a spec can say that a `.http` file cannot

- **A disabled row.** Optional parameters go into the comment block instead.
- **Multipart with per-part content types.** The format has no syntax for it.
- **Declarative captures and tests.** `.http` carries scripts, not
  `[[captures]]`/`[[tests]]` tables (see `docs/http-format.md`); response
  assertions belong in a `> {% … %}` block you write.
- **Typed api-key auth.** It is written as a header or a query parameter, which
  is the same bytes on the wire.
- **A per-request server.** There is one `{{baseUrl}}` per environment.
- **Several bodies for one request.** This is why named examples fan out into
  separate `###` blocks instead of one block with alternatives — the format has
  one body per request, and the fan-out is the honest encoding of that.
- **A saved example response.** `.http` has no place to store one, and inventing
  a Mándalo-only directive for it would break the interoperability that made the
  format worth adopting. So a response example is prose in a comment.

## Multiple servers

Each server becomes its own environment, named `<title>-<server description>` (or
`<title>-<host>` when the server has no description), so
`mandalo run petstore --env Petstore-Staging` picks one. Every environment gets
the same parameter seeds and the same secret declarations, so switching
environments changes the host and nothing else. A spec with no `servers` gets one
environment with an empty `baseUrl` and a warning.

## Re-importing a changed spec

**An import never touches a collection that already exists.** Importing the same
spec twice creates `petstore` and then `petstore-2`, and the summary names the
collection it wrote:

> `OpenAPI 3.0 imported into the collection "petstore-2". …`

This is the one behaviour that cannot lose work. A spec has no idea which
requests you have since edited — the script you added, the body you fixed, the
`> {% … %}` block that captures a token — and a merge that guesses would
eventually guess wrong. So the import writes somewhere new, and you diff the two
directories with the tool you already use for that.

A real two-way sync is a bigger feature than this one, and it needs three things
this importer does not have:

1. **Provenance.** Each request would have to record the `(path, method)` it came
   from and a hash of what the importer generated, so a later import can tell
   "unchanged since import" from "edited by hand".
2. **A three-way merge per request.** Spec-old vs spec-new vs on-disk, applied
   per field: update the URL and the mechanical headers, keep scripts, captures,
   edited bodies and hand-added headers, and report every conflict by name rather
   than resolving it.
3. **A deletion policy.** An operation that disappeared from the spec must be
   reported, never deleted — the file may be the only copy of a script.

Until that exists, `import` is one-way and additive, which is the version that
cannot be wrong.

## The report

Every import returns the same shape the Postman importer returns:

```
7 requests, 1 collections, 2 environments imported
warning  createPet: the operation is tagged pets, store — it was filed under "pets" only, because a request lives in one folder
warning  createPet: the spec names 3 request examples — one request was imported for each (full, minimal, rescue)
warning  readProfile: OAuth 2.0 ("OAuth2", the authorizationCode flow) imported without auth — …
warning  uploadPetPhoto: the form-data file fields (file) arrived empty — …
OpenAPI 3.0 imported into the collection "petstore". Path and required query parameters became {{variables}} seeded in the environment; importing the same spec again creates a new collection and never touches this one.
```

`imported` counts **requests written**, not operations — an operation with
several named examples contributes one per example. `warnings` is for things that
imported but are not what the spec said; `skipped` is for things that did not
import at all. Both name the operation. Neither is ever silent.

## Is the result usable?

For the common case — a REST spec with tags, a server URL, bearer auth, path and
query parameters, and JSON request bodies with schemas — yes: the collection
runs, the folders read like the spec's own sections, and the bodies are close
enough to edit rather than write. That is what the test suite asserts: a
Petstore-shaped spec is imported into a temporary workspace and executed through
the same `Runner` the CLI uses.

What it is not is a client generator. It will not run an OAuth flow, upload a
file, or assert on a response — those are yours to add once, in a file that the
next import will not overwrite.
