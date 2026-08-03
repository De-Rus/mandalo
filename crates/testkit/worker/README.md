# mandalo-mock worker

The hosted half of the mock API: the same HTTP and GraphQL surface as the Rust
mock in `crates/testkit`, running on Cloudflare Workers so the browser build of
Mándalo can make real requests out of the box.

- Worker name: **mandalo-mock**
- Route: `api.mandalo.dev/*` (DNS record points at the worker)
- Deployed by `.github/workflows/mock-worker.yml` on changes to this directory.

## What is not here

gRPC. Framing gRPC-Web in JavaScript would mean a second implementation of the
service, which is the one thing this repository refuses to do. gRPC lives in the
Rust mock: `make mock-api`, or the desktop app. `/mock.v1.*` answers 501 with
that explanation instead of failing silently.

## Contract

`../contract.json` is the single source of truth for the routes both
implementations serve. Run each side against it:

```sh
node --test crates/testkit/worker/contract.test.mjs   # the worker
cargo test -p mandalo-testkit --test contract         # the Rust mock
```

## Local run and deploy

```sh
npx wrangler dev                # http://localhost:8787
npx wrangler deploy             # needs CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID
```

## Caps

Body 1 MB · `/big?bytes=` ≤ 5 MB · `/slow?ms=` ≤ 10 s · `/redirect/:n` ≤ 10 hops.
`/redirect-to?url=` only ever answers with a redirect — the worker never fetches a
user-supplied URL, so it cannot be used as a proxy. Rate limiting is left to the
Cloudflare edge (WAF rate limiting rules on `api.mandalo.dev`), not implemented
in the worker.
