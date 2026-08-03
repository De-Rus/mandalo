FROM rust:1-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src-tauri/Cargo.toml ./src-tauri/Cargo.toml
RUN cargo build --release -p mandalo-testkit --bin mandalo-mock

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/mandalo-mock /usr/local/bin/mandalo-mock
ENV BIND=0.0.0.0 PORT=8080 GRPC_PORT=8081
EXPOSE 8080 8081
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/mandalo-mock"]
