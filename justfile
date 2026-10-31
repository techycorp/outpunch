
default:
	@just --list | lolcat

build:
	cargo build

test:
	cargo test

test-core:
	cargo test -p outpunch

test-axum:
	cargo test -p outpunch-axum

test-client:
	cargo test -p outpunch-client

check: vendor-claims
	cargo check

vendor-claims:
	uv run --script ~/.claude/skills/verify-vendor-claims/scripts/check_vendor_claims.py

test-vendor:
	cargo test -p outpunch-client --test vendor_rustls -- --ignored

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

lint: fmt-check clippy

test-all: test test-python test-node

coverage:
	cargo tarpaulin -p outpunch -p outpunch-axum -p outpunch-client --out stdout

# Python bindings
build-python:
	just -f bindings/python/justfile build

dev-python:
	just -f bindings/python/justfile dev

test-python:
	just -f bindings/python/justfile test

# Node.js bindings
build-node:
	just -f bindings/node/justfile build

test-node:
	just -f bindings/node/justfile test
