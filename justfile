
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
	cargo test -p outpunch --test vendor_tokio -- --ignored
	cd bindings/ruby && bundle exec rake compile && bundle exec rspec --tag vendor
	cd adapters/ruby/rack && bundle exec rspec --tag vendor
	cd bindings/node && npx vitest run --config vitest.vendor.config.ts

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

# TLA+ models (spec/); see spec/README.md
tla cfg="Tunnel" *args:
	scripts/tla/tlc spec/Tunnel.tla spec/{{cfg}}.cfg {{args}}

tla-all:
	scripts/tla/check-all spec/models.txt

# Check the TLA+ properties catch known bugs (spec/mutations.txt)
tla-mutants:
	scripts/tla/mutants spec/mutations.txt

# Check that executions of the core's tests are behaviors of the model (trace validation)
tla-traces:
	python spec/trace/check_traces.py

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
