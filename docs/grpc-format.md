# The `.grpc` format

Mándalo stores gRPC requests in `.grpc` files. The shape is the
[`.http` format](http-format.md) with three differences: the request line names
a gRPC call instead of a URL, header lines are metadata, and there is exactly
one reserved key. Everything else — `###` separators, `@vars`, comments,
`{% … %}` scripts, request identity — is the same, and is documented there.

## A file

```grpc
### Say hello
{{grpcUrl}}/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: mandalo

{"text": "hola", "count": 21}

> {%
pm.test("doubled", function () { pm.expect(pm.response.json().doubled).to.equal(42); });
%}
```

## The request line

```
<target>/<package.Service>/<Method>
```

That is not an invention. The gRPC wire protocol addresses a call as
`/package.Service/Method` on the target host, so the line is the real endpoint
URL — the same string that goes out on the wire. It follows that it interpolates
like any other URL (`{{grpcUrl}}` above), and that you can paste it from a proto
file or a log without translating it into three fields.

The alternative was three keys — `url:`, `service:`, `method:` — which reads as
configuration rather than as an address, and would have made the format's one
structural line indistinguishable from metadata. Those three keys are refused
outright for that reason; see below.

The line is split from the right: last segment is the method, the one before it
is the fully-qualified service, everything before that is the target.

| Written | Message |
| --- | --- |
| `mock.v1.Mock/Say` | ``line 2: "mock.v1.Mock/Say" names no target — write `target/package.Service/Method` `` |
| `{{grpcUrl}}` | ``line 2: expected `target/package.Service/Method`, found "{{grpcUrl}}"`` |
| `{{grpcUrl}}//Say` | ``line 2: this call line has no service`` |
| `{{grpcUrl}}/mock.v1.Mock/Say hello` | ``line 2: the method must not contain whitespace: "Say hello"`` |
| a `###` block with no call line | ``line 2: this block has no call line — a request needs `target/package.Service/Method` `` |

## Metadata, and the one reserved key

Every header line is gRPC metadata and is sent as written, except `proto:`.

```grpc
proto: protos/mock.proto
proto: protos/common.proto
x-trace: mandalo
```

`proto:` is repeatable — a call whose messages span several files lists them all.
Each value is workspace-relative, resolved against the directory holding
`mandalo.toml`, and confined to it. A collection is shared, untrusted
configuration, so an absolute path or a `..` escape is a hard stop:

> ``line 3: a `proto:` path must be a workspace-relative path, not "/tmp/mandalo-mock/mock.proto"``

> ``line 3: a `proto:` path must stay inside the workspace: "../../etc/x.proto"``

Proto paths interpolate, but only from the file's own `@vars`. A path that still
holds a `{{template}}` once those are applied — one that depends on the
environment — is refused, because it cannot be proven to stay inside the
workspace until the request is already going out:

> `grpc.grpc: a proto path that is only known at send time cannot be checked against the workspace — write "{{protoDir}}/mock.proto" as a workspace-relative path`

So this works:

```grpc
@protoDir = protos

### Say hello
{{grpcUrl}}/mock.v1.Mock/Say
proto: {{protoDir}}/mock.proto
```

and the same line with `protoDir` coming from an environment does not.

### The closed set of refused keys

Two groups of keys are refused instead of being sent as metadata.

| Key | Why |
| --- | --- |
| anything starting with `grpc-` | The protocol reserves that prefix for itself. Sending your own `grpc-timeout` would collide with the transport. |
| `url`, `service`, `method`, `message`, `protos`, `import`, `proto-path` | Each reads like a Mándalo directive. Sent as metadata they would look like they worked, and the call would go somewhere else entirely. |

```
line 4: gRPC reserves the `grpc-` metadata prefix for the protocol itself, so "grpc-timeout" cannot be sent
```

```
line 4: "service" reads like a Mándalo directive but would be sent as gRPC metadata — the only reserved key is `proto:`, and the call target lives on the request line
```

The list is closed on purpose. A key not on it is metadata, and stays metadata
however Mándalo grows: nothing you write today can be reinterpreted as a
directive tomorrow.

## The message

Everything after the blank line is the request message as JSON, verbatim, until
the next `###`, a `> {%` script, or the end of the file. A request with no
message body sends `{}`.

```grpc
### gRPC GetUser
{{grpcUrl}}/mock.v1.Mock/GetUser
proto: protos/mock.proto

{"id": "u-1", "tags": ["a", "b"], "tier": "TIER_PRO"}
```

Enums are written as their proto name, repeated fields as JSON arrays, nested
messages as nested objects — the standard protobuf JSON mapping. The proto is
compiled at send time, so a field that does not exist fails there, not here.

## Scripts

Same as `.http`: `< {% … %}` before the call line, `> {% … %}` after the message,
both running the `pm.*` engine described in
[postman-compatibility.md](postman-compatibility.md).

One caveat: a gRPC response has no HTTP status, so `pm.response.to.have.status`
has nothing to assert on. Assert on the decoded message through
`pm.response.json()`.

## Streaming

Streaming methods parse — a `.grpc` file can name one — and fail at send time
with the message the transport has always given:

> `streaming methods not supported yet`

## What the format cannot express

| Not expressible | What happens |
| --- | --- |
| An HTTP or GraphQL request | ``a .grpc file holds gRPC requests, not "http"`` |
| Declarative `[[tests]]` and `[[captures]]` | ``a .grpc file cannot carry declarative tests or captures — assert and capture from a `> {% … %}` response script instead`` |
| A per-request description | ``a .grpc file has no line for a description — put it in a `#` comment above the request`` |
| Server reflection instead of a `proto:` file | Not implemented — there is nothing to lose on save. Point at the `.proto`. |

Saving a request that needs any of the first three fails with the message shown.
Nothing is dropped in silence.
