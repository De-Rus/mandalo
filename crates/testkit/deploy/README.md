# Hosting the full mock on fly.io — OPTIONAL

The mock the browser app talks to is the Cloudflare Worker in
`crates/testkit/worker` (HTTP + GraphQL, nothing to patch, scale to zero, no
machine to watch). This directory is the other option: the **whole** Rust mock,
gRPC and gRPC-Web included, on a fly.io machine. Use it only if something needs
hosted gRPC.

## Deploy

```sh
cd crates/testkit/deploy
fly launch --no-deploy --copy-config --name mandalo-mock --region cdg
fly deploy --dockerfile Dockerfile --config fly.toml \
  --build-arg CONTEXT=../../..   # the build context is the repository root
```

The Dockerfile expects the repository root as its context, so from the root:

```sh
fly deploy --config crates/testkit/deploy/fly.toml --dockerfile crates/testkit/deploy/Dockerfile .
```

## Custom domain

```sh
fly certs add api.mandalo.dev -a mandalo-mock
fly certs show api.mandalo.dev -a mandalo-mock     # prints the DNS records to create
```

Point `api.mandalo.dev` at the app (A/AAAA to the machine IPs from
`fly ips list -a mandalo-mock`, or a CNAME to `mandalo-mock.fly.dev`). Only one of
the Worker and this app can own that hostname — pick one.

## Ports

| Port | What                                              |
|------|---------------------------------------------------|
| 443  | HTTP + GraphQL (`internal_port` 8080)             |
| 8443 | gRPC and gRPC-Web over TLS (`internal_port` 8081) |

## Health

`GET /health` → `200 {"status":"ok","version":"0.1.0"}`. That is what the fly
health check hits.

## Configuration

Everything is env driven — the same binary `make mock-api` runs locally:

| Variable                | Default     | What                                  |
|-------------------------|-------------|---------------------------------------|
| `BIND`                  | `127.0.0.1` | bind address                          |
| `PORT`                  | `8787`      | HTTP port                             |
| `GRPC_PORT`             | `50051`     | gRPC / gRPC-Web port                  |
| `RATE_LIMIT_PER_MINUTE` | `600` when the bind address is not loopback | per client address |
