# mandalo-testkit

The mock API Mándalo is tested against, and the one you can click around by hand.

```sh
make mock-api          # http://localhost:8787, gRPC on 50051, Ctrl-C to stop
```

It prints its base URL, the path of the generated `mock.proto` and an index of
every endpoint, then logs one line per request (method, path, status, duration —
never bodies, never Authorization headers).

## In a test

```rust
let api = MockApi::start().await;          // ephemeral ports, stops when dropped
api.url("/get");                           // http://127.0.0.1:54321/get
api.grpc_url();                            // http://127.0.0.1:54322
api.proto_path();                          // /tmp/.../mock.proto
api.received_requests();                   // what actually reached the wire
```

`fixtures::` builds the workspaces and requests the integration suites use, and
`fixtures::install_example_workspace` copies `examples/mock-workspace` into a
tempdir pointed at a running mock.

## Endpoints

| Area | Routes |
|---|---|
| methods | `/get` `/post` `/put` `/patch` `/delete` `/head` `/options` — each echoes method, path, query, headers, body |
| status | `/status/:code`, `/redirect/:n`, `/redirect-to?url=`, `/slow?ms=` |
| auth | `POST /auth/login`, `/auth/basic`, `/auth/bearer`, `/auth/apikey` |
| bodies | `/json` `/xml` `/text` `/empty` `/binary` `/gzip` `/brotli` `/big?bytes=` `/headers/echo` `/cookies/set` |
| body echoes | `POST /body/multipart` → every part in order (`name`, `filename`, `contentType`, `size`, `text`, `bytes`), never grouped by name · `POST /body/urlencoded` → ordered `pairs` plus the raw string · `POST /body/binary` → `size`, `contentType`, `bytes` |
| service | `/health` → `{status, version}` |
| graphql | `POST /graphql` — user, users, mutation, an `errors` array with HTTP 200, a malformed body |
| grpc | `mock.v1.Mock`: `Say`, `GetUser`, `Fail`, `Slow`, `Ticks` (server streaming), native and gRPC-Web |

Credentials: `ada` / `lovelace`, bearer `mock-bearer-token`, `x-api-key: mock-api-key`.

## Caps

It is also deployed publicly, so it is capped everywhere it generates or waits:

| Cap | Limit |
|---|---|
| request body | 1 MB → 413 |
| `/big?bytes=` | 5 MB |
| `/slow?ms=` and gRPC `Slow` | 10 s |
| `/redirect/:n` | 10 hops |
| rate limit | 600 requests per minute per client address, on when the bind address is not loopback |

`/redirect-to?url=` answers with a redirect and never fetches the URL itself, so
the mock cannot be used as a proxy.

## The two implementations

`crates/testkit/worker` is a Cloudflare Worker serving the same HTTP and GraphQL
routes for the browser build (gRPC stays here). `contract.json` pins the routes
both must serve, and both are tested against it.
