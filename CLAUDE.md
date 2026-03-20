# CLAUDE.md

Outpunch is a reverse WebSocket tunnel proxy written in Rust. It lets you expose services on a private network through a public-facing server without opening inbound ports. A client on the private network connects outbound via WebSocket to the server, which relays HTTP requests through the tunnel and returns responses. The core is a framework-agnostic Rust library that handles all tunnel logic — protocol parsing, request/response coordination, connection state, authentication — through plain types and message channels, with no dependency on any web framework or WebSocket library.

The project ships a standalone server binary, a standalone client binary, and thin server framework adapters (axum first) that translate between a web framework's types and the core. Language bindings for the client use per-language FFI tools (PyO3 for Python, Napi-RS for Node.js, Magnus for Ruby) rather than a single generator — each gets a thin wrapper, not a reimplementation.

## Conventions

### Test files are separate from source files

Unit tests go in `_tests.rs` files next to the source, not inline. This allows opening tests and code side-by-side in editor tabs.

```
src/
  protocol.rs           # code
  protocol_tests.rs     # tests
```

The source file links to the test file at the bottom:
```rust
#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
```

### Dependencies

Always use the package manager (`cargo add`, `cargo remove`) to manage dependencies. Never hand-edit Cargo.toml dependency sections.

## Documentation

- [docs/product.md](docs/product.md) — Product overview, deployment modes, protocol spec, authentication model, integration strategy
- [docs/architecture.md](docs/architecture.md) — Crate structure, core design (why it exists, plain types, channel-based WS interface), server framework adapter pattern, dependencies
- [docs/testing.md](docs/testing.md) — Testing strategy: adapter conformance suite, core unit tests, integration tests, failure/edge cases, fuzz testing
- [docs/glossary.md](docs/glossary.md) — Term definitions: core, server framework adapter, tunnel client, tunnel request/response, standalone/embedded modes
- [docs/project-structure.md](docs/project-structure.md) — Repo layout: Rust crates, language bindings directory structure, FFI tools per language, build/release strategy
- [docs/tunnel-proxy.md](docs/tunnel-proxy.md) — Original design doc with full architecture, protocol spec, and reference Ruby/Python implementation
