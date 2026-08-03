# Mock API workspace

A ready-made Mándalo workspace pointed at the mock API. Open this directory as a
workspace in the desktop app, or run it from the terminal:

```sh
make mock-api                                  # starts the mock on localhost:8787
mandalo --workspace "$PWD/examples/mock-workspace" run mock --env local
```

## Environments

| Environment | `baseUrl`                  | What runs                          |
|-------------|----------------------------|------------------------------------|
| `local`     | `http://localhost:8787`    | everything, gRPC included          |
| `hosted`    | `https://api.mandalo.dev`  | HTTP and GraphQL (no gRPC)         |

The hosted mock is a Cloudflare Worker: it speaks HTTP and GraphQL only, so the
two gRPC requests and the three `upload.http` requests need the local mock
(`make mock-api`). They run in the desktop app and in the browser build alike —
the mock's gRPC port serves native gRPC and gRPC-Web side by side, and the browser
compiles the proto with the same protox compiler built to WebAssembly.

`--workspace` wants an absolute path, so from the repository root:

```sh
mandalo --workspace "$PWD/examples/mock-workspace" run mock --env local
```

## Files

Requests are plain-text `.http` and `.grpc` files, one per theme. Each file holds
several requests separated by `###`, and the text after `###` is the request name.
The formats are described in [`docs/http-format.md`](../../docs/http-format.md)
and [`docs/grpc-format.md`](../../docs/grpc-format.md).

- `http.http` — one request per method, plus JSON, headers, status, redirects,
  gzip, binary and a slow response. It opens with a file-scoped `@trace`
  variable, which beats a variable of the same name in the environment.
- `auth.http` — Basic, Bearer and API key against their guarded endpoints.
- `graphql.http` — a query with variables, a list query, and the classic trap: a
  GraphQL `errors` array arriving with HTTP 200. Each is marked with the
  `X-REQUEST-TYPE: GraphQL` header, which is consumed and never sent.
- `chain.http` — login, capture the token with `pm.environment.set` in a
  `> {% … %}` response script, then use it on the next request.
- `upload.http` — the body modes that carry form pairs or files: urlencoded
  written as text, and two `< ./files/…` file bodies whose `Content-Type` comes
  from the extension. Needs the local mock.
- `grpc.grpc` — unary calls with metadata, nested messages, repeated fields and
  enums.
- `protos/mock.proto` — the proto the `grpc.grpc` requests compile.
- `files/` — the fixtures `upload.http` sends. Body and `proto:` paths are always
  relative to the workspace root, and may not leave it.

The gRPC requests name the proto as `proto: protos/mock.proto`, a
workspace-relative path — the only kind either format accepts. On the desktop it
is read from disk; in the browser it is matched by file name against the
workspace `protos/` folder. Use “Proto files” in the web ribbon to add your own.

## Running one request

A request is addressed as `<file>#<index>`, counting from zero:

```sh
mandalo send mock http.http#2 --workspace "$PWD/examples/mock-workspace" --env local
```

A name works too, as long as it is unique in the file. It reads better and breaks
when you rename the request:

```sh
mandalo send mock "http.http#Teapot" --workspace "$PWD/examples/mock-workspace" --env local
```

`mandalo ls --workspace "$PWD/examples/mock-workspace"` prints every request with
the address to use.
