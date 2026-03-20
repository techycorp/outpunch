
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

check:
	cargo check

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

lint: fmt-check clippy

coverage:
	cargo tarpaulin --workspace --out stdout

# Python bindings
build-python:
	just -f bindings/python/justfile build

dev-python:
	just -f bindings/python/justfile dev

test-python:
	just -f bindings/python/justfile test
