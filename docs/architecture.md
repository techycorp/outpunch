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

### Why the Core Exists

The tunnel has two routing problems — two sides of the same coin:

1. **Inbound (service routing)**: an HTTP request arrives at the server — which *client* should handle it? Solved by the service name in the URL path.
2. **Outbound (response matching)**: a response arrives on the WebSocket — which *HTTP handler* is waiting for it? Solved by matching the `request_id`.

Both happen over the same WebSocket connection. Multiple HTTP requests can be in-flight simultaneously, all waiting for responses. Even with a single client and a single service, two users hitting the tunnel endpoint at the same time need their responses delivered to the right caller.

The **pending request map** is what makes response matching work. When an HTTP request arrives, the core assigns a unique `request_id`, parks a waiting handler in the map, and sends the request over the WebSocket. When a response comes back, the core looks up the `request_id`, finds the right waiting handler, and delivers the response. If no response arrives, the core times out and cleans up.

This coordination has real concurrency hazards: race conditions between timeouts and responses arriving simultaneously, cleanup when a client disconnects mid-request, atomic connection state transitions so a stale disconnect doesn't clobber a fresh connection. Getting this wrong means hung requests, leaked resources, or silent data loss.

Writing this once in Rust — where the type system enforces thread safety and ownership — means every adapter and every language binding inherits correct behavior. The alternative is reimplementing this coordination logic in every language, with each implementation carrying its own concurrency bugs.

Everything else the core provides (protocol types, auth, header filtering) is straightforward. The request/response coordination is the reason the core exists.

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

The core does **not** own: HTTP routing, WebSocket upgrade mechanics, WebSocket I/O, or anything framework- or library-specific.

### Channel-Based WebSocket Interface

The core never touches a WebSocket type directly. Instead, it communicates through **message channels** — it receives incoming messages from one channel and sends outgoing messages through another:

```rust
// The core's interface for a WebSocket connection
server.handle_connection(incoming_rx, outgoing_tx).await;
```

The core reads strings from `incoming_rx` and writes strings to `outgoing_tx`. It doesn't know or care what's on the other end — tungstenite, a Python WebSocket library, a Ruby one, or a test harness feeding it fake messages.

**This is what makes outpunch truly multi-language.** The adapter (or language binding) is responsible for bridging the actual WebSocket to these channels:

- **Rust adapter (e.g., axum)**: upgrades HTTP → tungstenite `WebSocketStream`, runs a small bridge loop (~10 lines) that reads from the stream into `incoming_tx` and writes from `outgoing_rx` to the stream.
- **Python binding**: reads from Python's WS library, feeds messages through FFI into `incoming_tx`. Same in reverse.
- **Ruby binding**: same pattern with Ruby's WS library.
- **Test harness**: feeds scripted messages directly into the channels — no network needed.

The bridge loop is mechanical boilerplate. All the logic — auth, message routing, pending request matching, timeouts — lives in the core, operating on plain strings through channels.

## Server Framework Adapter

The adapter is the only part of outpunch that touches a web framework. It handles two HTTP-level concerns and a WebSocket bridge:

1. **Tunnel endpoint** (`/tunnel/*path`) — translates the framework's HTTP request into a `TunnelRequest`, calls the core, translates the `TunnelResponse` back into the framework's HTTP response.
2. **WebSocket upgrade** (`/ws`) — uses the framework's upgrade mechanism to establish a WebSocket connection.
3. **WebSocket bridge** — a small loop that pipes messages between the framework's WebSocket stream and the core's message channels.

After the bridge is set up, the adapter is no longer involved in the WebSocket logic. The core handles authentication, message routing, pending request tracking, and heartbeat through the channels. The adapter only exists at the HTTP boundary.

This is why adapters stay thin: they're type translation plus a bridge loop. All logic lives in the core.

For example, `outpunch-axum` would:

- Provide an axum `Router` with the catch-all `/tunnel/*path` route and `/ws` upgrade endpoint
- Convert `axum::extract::Request` → `TunnelRequest`
- Convert `TunnelResponse` → `axum::response::Response`
- On WS upgrade: bridge the tungstenite stream to the core's message channels

The standalone server binary uses `outpunch-axum` internally — it's the same adapter a user would use to embed outpunch in their own axum app.

Adding support for a new framework or language means writing a new adapter. No core changes.

## Language Bindings (Future)

Each language binding (via UniFFI or similar) exposes the core, and per-framework adapters follow the same pattern — translate HTTP types and bridge WebSocket messages:

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
| `tokio` | async runtime (including message channels) | core, server, client |
| `serde` / `serde_json` | protocol serialization | core |
| `uuid` | request ID generation | core |
| `tokio-tungstenite` | WebSocket (server bridge + client connection) | axum adapter, client binary |
| `reqwest` | HTTP client (forwarding requests to local services) | client binary |

Note: `tokio-tungstenite` is **not** a dependency of the core — only of adapters and the client binary. The core communicates through tokio channels only.

The first server framework adapter will target **axum**. The core itself has no framework or WebSocket library dependency — additional adapters can be added later without core changes.

## Standalone vs Embedded

**Standalone**: `outpunch-server` binary runs its own process on its own port. The host app reverse-proxies `/tunnel/*` to it. Framework-agnostic — works with anything.

**Embedded**: the host app imports the adapter crate (e.g., `outpunch-axum`) and mounts the routes into its own server. Single process, no extra port, but couples to a specific framework's adapter.

Both modes use the same core logic. The standalone binary is just the adapter + a `main()` that starts the server.
