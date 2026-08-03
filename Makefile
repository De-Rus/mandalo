CARGO := cd src-tauri && cargo

.PHONY: help install dev build test test-rust test-web lint fmt clean landing release-check

help:
	@echo "Mándalo — make targets"
	@echo ""
	@echo "  make install      install frontend deps (pnpm)"
	@echo "  make dev          run the desktop app (hot reload)"
	@echo "  make test         run everything: Rust + frontend"
	@echo "  make test-rust    cargo test (core: http, graphql, grpc, files)"
	@echo "  make test-web     vitest (UI + stores + mapping)"
	@echo "  make lint         clippy -D warnings + tsc typecheck"
	@echo "  make fmt          cargo fmt"
	@echo "  make build        production desktop bundle (installers)"
	@echo "  make landing      preview the landing page at localhost:8788"
	@echo "  make release-check everything CI runs, before tagging"
	@echo "  make clean        remove build artifacts"

install:
	pnpm install

dev: install
	pnpm tauri dev

test: test-rust test-web

test-rust:
	$(CARGO) test

test-web: install
	pnpm test

lint:
	$(CARGO) clippy --all-targets -- -D warnings
	pnpm build

fmt:
	$(CARGO) fmt

build: install
	pnpm tauri build

landing:
	@echo "http://localhost:8788"
	@python3 -m http.server 8788 --directory landing

release-check: lint test
	@echo "OK — safe to tag"

clean:
	$(CARGO) clean
	rm -rf dist node_modules
