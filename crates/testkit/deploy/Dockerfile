FROM rust:1-slim AS build
WORKDIR /src
# build-essential and a full perl are for openssl-src: workspace feature
# unification pulls git2's vendored OpenSSL into this build even though the mock
# never uses it, and its Configure script needs both.
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config build-essential perl make \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# mandalo-core's build script embeds the sample workspace, so the crate does not
# compile without it.
COPY examples ./examples
COPY src-tauri/Cargo.toml ./src-tauri/Cargo.toml
# The desktop crate is a workspace member, so cargo parses its manifest even
# though nothing here builds it. Empty sources are enough to satisfy that.
RUN mkdir -p src-tauri/src && touch src-tauri/src/lib.rs src-tauri/src/main.rs
RUN cargo build --release -p mandalo-testkit --bin mandalo-mock

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/mandalo-mock /usr/local/bin/mandalo-mock
ENV BIND=0.0.0.0 PORT=8080 GRPC_PORT=8081
EXPOSE 8080 8081
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/mandalo-mock"]
