# CLAUDE.md

## What This Is

Outpunch is a Rust implementation of the reverse WebSocket tunnel proxy pattern described in `docs/tunnel-proxy.md`. The goal is a single Rust codebase that implements **both sides** of the tunnel:

- **Server** — the public-facing proxy that accepts HTTP requests, shuttles them through a WebSocket tunnel, and returns responses. (Currently implemented in Ruby/Rails in production.)
- **Client** — the private-server agent that connects outbound via WebSocket, receives proxied requests, forwards them to local services, and returns responses. (Currently implemented in Python in `tunnel-client/`.)

## Why Rust

The Python tunnel client works but is too thick — it reimplements protocol logic, connection management, and HTTP forwarding that should live in a shared core. The Ruby server side is tied to Rails/ActionCable. A Rust core lets us:

1. Write the tunnel logic once, correctly
2. Ship a standalone Rust server and client first
3. Later expose bindings to other languages (Python, Ruby, Node.js) via UniFFI or similar, so each language gets a thin wrapper — not a full reimplementation

## Current State

- `docs/tunnel-proxy.md` — full design doc with architecture, protocol spec, and reference Ruby implementation
- `tunnel-client/` — production Python tunnel client (multi-service, Docker Swarm deployment)
- No Rust code yet. Project is in the research/planning phase.

## Open Questions

- **Server-side integration**: How does the Rust server component integrate with different web frameworks? Middleware? Plugin? Standalone sidecar process? Framework-specific vs framework-agnostic?
- **Binding strategy**: UniFFI (Mozilla) is the leading candidate for multi-language bindings. Diplomat is an alternative. Need to research tradeoffs for this specific use case.
- **Protocol**: Should we keep ActionCable compatibility or define a simpler raw WebSocket protocol (as sketched in the docs' "Simplified Protocol" section)?
- **Scope of bindings**: What surface area do language bindings expose — just "start a client/server" or granular access to protocol types, message parsing, etc.?
