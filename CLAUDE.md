# CLAUDE.md

Outpunch is a reverse WebSocket tunnel proxy written in Rust. It lets you expose services on a private network through a public-facing server without opening inbound ports. A client on the private network connects outbound via WebSocket to the server, which relays HTTP requests through the tunnel and returns responses. The core is a framework-agnostic Rust library that handles all tunnel logic — protocol parsing, request/response coordination, connection state, authentication — through plain types and message channels, with no dependency on any web framework or WebSocket library.

The project ships a standalone server binary, a standalone client binary, and thin server framework adapters (axum first) that translate between a web framework's types and the core. The same adapter pattern extends to language bindings (Python, Ruby, Node.js via UniFFI) so each language gets a thin wrapper, not a reimplementation. The project is in the planning/documentation phase — no Rust implementation yet.

## Documentation

- [docs/product.md](docs/product.md) — Product overview, deployment modes, protocol spec, authentication model, integration strategy
- [docs/architecture.md](docs/architecture.md) — Crate structure, core design (why it exists, plain types, channel-based WS interface), server framework adapter pattern, dependencies
- [docs/testing.md](docs/testing.md) — Testing strategy: adapter conformance suite, core unit tests, integration tests, failure/edge cases, fuzz testing
- [docs/glossary.md](docs/glossary.md) — Term definitions: core, server framework adapter, tunnel client, tunnel request/response, standalone/embedded modes
- [docs/tunnel-proxy.md](docs/tunnel-proxy.md) — Original design doc with full architecture, protocol spec, and reference Ruby/Python implementation
