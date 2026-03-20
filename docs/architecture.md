# Outpunch — Architecture

## Design Principle: Framework-Agnostic Core

The core library (`outpunch`) has no dependency on any web framework. It defines its own plain types (method, path, headers, body as strings/maps) and handles all tunnel logic: protocol parsing, pending request tracking, WebSocket message handling, connection state.

Framework integration happens through **thin adapter crates** that translate between a specific framework's request/response types and outpunch's plain types. Each adapter is small (~20-50 lines of glue).

## Crate Structure

```
crates/
  outpunch/              # core library — protocol types, pending request map, WS handling
  outpunch-server/       # standalone server binary (uses outpunch-axum internally)
  outpunch-client/       # standalone client binary
  outpunch-axum/         # adapter: axum types <-> outpunch types
```

## Core Crate (`outpunch`)

### Framework-Free Types

The core defines its own request and response types using only basic Rust types — `String`, `HashMap`, `u16`, etc. No framework dependencies, no framework traits, no borrowed lifetimes tied to a specific HTTP library.

```rust
// What the core works with — just data
struct TunnelRequest {
    request_id: String,
    service: String,
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Option<String>,
}

struct TunnelResponse {
    request_id: String,
    status: u16,
    headers: HashMap<String, String>,
    body: Option<String>,
    body_encoding: Option<String>,
}
```

These same types are what gets serialized to/from JSON over the WebSocket. They're also what adapters translate to and from.

### Responsibilities

The core owns:

- Protocol message types (`Auth`, `AuthOk`, `TunnelRequest`, `TunnelResponse`)
- Message serialization/deserialization (serde, keyed on `type` field)
- Pending request map: track in-flight requests by `request_id`, wait for responses with timeout
- Connection state management (register/unregister, connected check, atomic compare-and-set on disconnect)
- Auth validation (constant-time shared secret comparison)
- Header filtering (strip hop-by-hop headers)

The core does **not** own: HTTP routing, WebSocket upgrade mechanics, or anything framework-specific.

## Server Framework Adapter

The adapter is the only part of outpunch that touches a web framework. It handles two HTTP-level concerns and nothing else:

1. **Tunnel endpoint** (`/tunnel/*path`) — translates the framework's HTTP request into a `TunnelRequest`, calls the core, translates the `TunnelResponse` back into the framework's HTTP response.
2. **WebSocket upgrade** (`/ws`) — uses the framework's upgrade mechanism to establish a WebSocket connection, then hands the raw stream to the core.

**After the WebSocket upgrade, the adapter is no longer involved.** The core owns the WebSocket connection entirely — authentication, message routing, pending request tracking, heartbeat. The adapter only exists at the HTTP boundary.

This is why adapters stay thin (~20-50 lines): they're just type translation. All logic lives in the core.

For example, `outpunch-axum` would:

- Provide an axum `Router` with the catch-all `/tunnel/*path` route and `/ws` upgrade endpoint
- Convert `axum::extract::Request` → `TunnelRequest`
- Convert `TunnelResponse` → `axum::response::Response`
- On WS upgrade: hand the resulting `WebSocket` stream to the core

The standalone server binary uses `outpunch-axum` internally — it's the same adapter a user would use to embed outpunch in their own axum app.

Adding support for a new Rust framework (e.g., actix-web) means writing a new adapter crate. No core changes.

## Language Bindings (Future)

Each language binding (via UniFFI or similar) exposes the core, and per-framework adapters follow the same pattern:

| Language | Adapter | Covers |
|----------|---------|--------|
| Ruby | `outpunch-rack` | Rails, Sinatra, any Rack app |
| Python | `outpunch-asgi` | Django, FastAPI, Starlette |
| Python | `outpunch-wsgi` | Flask |
| Node.js | `outpunch-express` | Express |
| Node.js | `outpunch-hono` | Hono |

## Rust Dependencies

### Decided

| Crate | Role | Used by |
|-------|------|---------|
| `tokio` | async runtime | core, server, client |
| `serde` / `serde_json` | protocol serialization | core |
| `uuid` | request ID generation | core |
| `tokio-tungstenite` | WebSocket client (outbound tunnel connection) | client binary |
| `reqwest` | HTTP client (forwarding requests to local services) | client binary |

The first server framework adapter will target **axum**. The core itself has no framework dependency — additional adapters (actix-web, etc.) can be added later without core changes.

## Standalone vs Embedded

**Standalone**: `outpunch-server` binary runs its own process on its own port. The host app reverse-proxies `/tunnel/*` to it. Framework-agnostic — works with anything.

**Embedded**: the host app imports the adapter crate (e.g., `outpunch-axum`) and mounts the routes into its own server. Single process, no extra port, but couples to a specific framework's adapter.

Both modes use the same core logic. The standalone binary is just the adapter + a `main()` that starts the server.
