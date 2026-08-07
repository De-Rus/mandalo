# Mock API workspace

A ready-made Mándalo workspace pointed at the mock API. Open this directory as a
workspace in the desktop app, or run it from the terminal:

```sh
make mock-api                                  # starts the mock on localhost:8787
mandalo --workspace "$PWD/examples/mock-workspace" run mock --env local
```

`run` sends the requests that have a response to assert on. The streams under
`streams/` are opened one at a time instead, because a connection that stays
open has no next request to move on to:

```sh
mandalo --workspace "$PWD/examples/mock-workspace" listen mock 'streams/chat.ws#0' \
  --env hosted --message hola --max 1
```

## Environments

| Environment | `baseUrl`                  | `wsUrl`                  | `mqttUrl`                       | What runs                            |
|-------------|----------------------------|--------------------------|---------------------------------|--------------------------------------|
| `local`     | `http://localhost:8787`    | `ws://localhost:8787`    | `mqtt://localhost:1883`         | everything, gRPC included            |
| `hosted`    | `https://api.mandalo.dev`  | `wss://api.mandalo.dev`  | `wss://api.mandalo.dev:8884`    | everything but gRPC                  |

The hosted mock is a Cloudflare Worker: it speaks HTTP and GraphQL only, so the
gRPC requests and the `uploads/bodies.http` requests need the local mock
(`make mock-api`). They run in the desktop app and in the browser build alike —
the mock's gRPC port serves native gRPC and gRPC-Web side by side, and the browser
compiles the proto with the same protox compiler built to WebAssembly.

`--workspace` wants an absolute path, so from the repository root:

```sh
mandalo --workspace "$PWD/examples/mock-workspace" run mock --env local
```

## Files

Requests are plain-text `.http`, `.grpc`, `.ws` and `.mqtt` files, one per theme. Each file holds
several requests separated by `###`, and the text after `###` is the request name.
The formats are described in [`docs/http-format.md`](../../docs/http-format.md),
[`docs/grpc-format.md`](../../docs/grpc-format.md) and
[`docs/stream-formats.md`](../../docs/stream-formats.md).

- `http/echo.http` — one request per method, plus JSON, headers, status, redirects,
  gzip, binary and a slow response. It opens with a file-scoped `@trace`
  variable, which beats a variable of the same name in the environment.
- `auth/methods.http` — Basic, Bearer and API key against their guarded endpoints.
- `graphql/queries.http` — a query with variables, a list query, and the classic trap: a
  GraphQL `errors` array arriving with HTTP 200. Each is marked with the
  `X-REQUEST-TYPE: GraphQL` header, which is consumed and never sent.
- `chaining/login-flow.http` — a pre-request script sets the login credentials,
  the response script captures the token with `pm.environment.set`, and the next
  request's pre-script upserts `Authorization: Bearer …` from that token.
- `uploads/bodies.http` — the body modes that carry form pairs or files:
  urlencoded written as text, two `< ./files/…` file bodies whose `Content-Type`
  comes from the extension, and a `multipart/form-data` body. Several files
  under one key are one line (`attachments = < ./a < ./b`) — what a browser
  posts for `<input type="file" multiple>`, and what the workbench shows as
  chips under a single key. Needs the local mock.
- `streams/chat.ws` — three websockets against the mock's echo route: one with
  named messages to send, one that negotiates a subprotocol, one that reconnects
  and pings. `mandalo listen mock 'streams/chat.ws#0' --env local --message hola`.
- `streams/events.http` — server-sent events. There is no `.sse` file type: an
  SSE request is an HTTP GET that sends `Accept: text/event-stream`, so it lives
  in a `.http` file with every other HTTP request. The second one turns the
  automatic resume off with `# @reconnect off`.
- `streams/sensors.mqtt` — an MQTT connection with a subscription and two
  publishes, one of them retained. The topics carry `{{room}}`, which comes from
  the environment. Locally it needs the broker `make mock-api` now starts on
  1883 (websockets on 1884).
- `grpc/service.grpc` — unary calls with metadata, nested messages, repeated fields and
  enums.
- `protos/mock.proto` — the proto the gRPC requests compile.
- `files/` — the fixtures `uploads/bodies.http` sends. Body and `proto:` paths are always
  relative to the workspace root, and may not leave it.

The gRPC requests name the proto as `proto: protos/mock.proto`, a
workspace-relative path — the only kind either format accepts. On the desktop it
is read from disk; in the browser it is matched by file name against the
workspace `protos/` folder. Use “Proto files” in the web ribbon to add your own.

## Running one request

A request is addressed as `<file>#<index>`, counting from zero:

```sh
mandalo send mock http/echo.http#2 --workspace "$PWD/examples/mock-workspace" --env local
```

A name works too, as long as it is unique in the file. It reads better and breaks
when you rename the request:

```sh
mandalo send mock "http/echo.http#Teapot" --workspace "$PWD/examples/mock-workspace" --env local
```

`mandalo ls --workspace "$PWD/examples/mock-workspace"` prints every request with
the address to use.
