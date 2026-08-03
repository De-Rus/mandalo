TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')

.PHONY: help install dev build cli cli-dist installer-check test test-rust test-e2e test-web lint fmt clean landing mock-api wasm release-check

help:
	@echo "Mándalo — make targets"
	@echo ""
	@echo "  make install      install frontend deps (pnpm)"
	@echo "  make dev          run the desktop app (hot reload)"
	@echo "  make test         run everything: Rust + frontend"
	@echo "  make test-rust    cargo test --workspace (core + cli + tauri)"
	@echo "  make test-e2e     integration suite against the local mock API"
	@echo "  make cli          build the mandalo command line binary (size-tuned profile)"
	@echo "  make cli-dist     package that binary the way a release does"
	@echo "  make installer-check  lint install.sh and prove the landing copy is in sync"
	@echo "  make test-web     vitest (UI + stores + mapping)"
	@echo "  make lint         clippy -D warnings + tsc typecheck"
	@echo "  make fmt          cargo fmt"
	@echo "  make build        production desktop bundle (installers)"
	@echo "  make landing      preview the landing page at localhost:8788"
	@echo "  make mock-api     run the local mock API for manual testing"
	@echo "  make wasm         rebuild the browser proto compiler (src/lib/web/protoc)"
	@echo "  make release-check everything CI runs, before tagging"
	@echo "  make clean        remove build artifacts"

install:
	pnpm install

dev: install
	pnpm tauri dev

test: test-rust test-web

test-rust:
	cargo test --workspace

test-e2e:
	cargo test -p mandalo-testkit --test contract
	cargo test -p mandalo-core --test e2e_http --test e2e_graphql --test e2e_grpc --test e2e_pipeline
	cargo test -p mandalo-cli --test e2e_cli
	node --test crates/testkit/worker/contract.test.mjs

test-web: install
	pnpm test

lint:
	cargo clippy --workspace --all-targets -- -D warnings
	pnpm build

fmt:
	cargo fmt --all

build: install
	pnpm tauri build

cli:
	cargo build --profile release-cli -p mandalo-cli
	@ls -l target/release-cli/mandalo

cli-dist:
	cargo build --profile release-cli -p mandalo-cli --target $(TARGET)
	@scripts/package-cli.sh $(TARGET) >/dev/null

installer-check:
	shellcheck -s sh scripts/install.sh
	shellcheck scripts/package-cli.sh scripts/release-notes.sh scripts/sync-packaging.sh
	@cmp scripts/install.sh landing/install.sh \
	  || (echo "landing/install.sh is stale — run: cp scripts/install.sh landing/install.sh" && exit 1)
	@echo "installer OK"

mock-api:
	cargo run -p mandalo-testkit --bin mandalo-mock

wasm:
	cd crates/grpc-wasm && wasm-pack build --target web --out-dir ../../src/lib/web/protoc --out-name protoc
	rm -f src/lib/web/protoc/.gitignore src/lib/web/protoc/package.json
	@ls -l src/lib/web/protoc/protoc_bg.wasm

landing:
	@echo "http://localhost:8788"
	@python3 -m http.server 8788 --directory landing

release-check: lint test installer-check
	@echo "OK — safe to tag"

clean:
	cargo clean
	rm -rf dist node_modules
