# Outpunch — Architecture

## Design Principle: Framework-Agnostic Core

The core library (`outpunch`) has no dependency on any web framework. It defines its own plain types (method, path, headers, body as strings/maps) and handles all tunnel logic: protocol parsing, pending request tracking, WebSocket message handling, connection state.

Framework integration happens through **thin adapter crates** that translate between a specific framework's request/response types and outpunch's plain types. Each adapter is small (~20-50 lines of glue).

## Crate Structure

```
crates/
  outpunch/              # core library — protocol types, pending request map, WS handling
  outpunch-axum/         # adapter: axum types <-> outpunch types
  outpunch-client/       # client library + binary
bindings/
  python/                # Python bindings via PyO3
```

See [project-structure.md](project-structure.md) for the full layout including language bindings.

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

### Connection-Based WebSocket Interface

The core never touches a WebSocket type directly. Instead, it provides a `Connection` object that adapters interact with through simple method calls:

```rust
// Adapter creates a connection
let connection = server.create_connection();

// Adapter pushes incoming WS messages into the connection
connection.push_message(text).await;

// Adapter registers a callback for outgoing messages
connection.on_message(|msg| { /* send on WS */ });

// Core runs the connection lifecycle (auth, request routing, etc.)
connection.run().await;
```

The `Connection` owns the message channels internally. The adapter doesn't create or manage channels — it just calls `push_message` when a WS message arrives and handles `on_message` callbacks to send messages back. It doesn't know or care what's happening inside — auth, pending request matching, timeouts are all handled by the core.

**This is what makes outpunch truly multi-language.** The adapter (or FFI binding) only needs to wrap two methods:

- **Rust adapter (e.g., axum)**: calls `push_message` from the WS read loop, sends messages from `on_message` to the WS write side.
- **JS adapter (via Napi-RS)**: wraps `push_message` as a method call, wraps `on_message` as a `ThreadsafeFunction` callback. Two thin bindings.
- **Test harness**: calls `push_message` directly with scripted messages — no network needed.

All logic — auth, message routing, pending request matching, timeouts — lives inside the `Connection`, behind a simple push/callback interface.

## Server Framework Adapter

The adapter is the only part of outpunch that touches a web framework. It handles two HTTP-level concerns and a WebSocket bridge:

1. **Tunnel endpoint** (`/tunnel/*path`) — translates the framework's HTTP request into an `IncomingRequest`, calls `server.handle_request()`, translates the `TunnelResponse` back into the framework's HTTP response.
2. **WebSocket upgrade** (`/ws`) — uses the framework's upgrade mechanism to establish a WebSocket connection.
3. **WebSocket bridge** — pipes WS messages into `connection.push_message()` and sends `on_message` callbacks back through the WS. No channel management — the `Connection` handles that internally.

After the bridge is set up, the adapter is no longer involved in the WebSocket logic. The core handles authentication, message routing, pending request tracking, and heartbeat through the channels. The adapter only exists at the HTTP boundary.

This is why adapters stay thin: they're type translation plus a bridge loop. All logic lives in the core.

For example, `outpunch-axum` would:

- Provide an axum `Router` with the catch-all `/tunnel/*path` route and `/ws` upgrade endpoint
- Convert `axum::extract::Request` → `TunnelRequest`
- Convert `TunnelResponse` → `axum::response::Response`
- On WS upgrade: bridge the tungstenite stream to the core's message channels

The standalone server binary uses `outpunch-axum` internally — it's the same adapter a user would use to embed outpunch in their own axum app.

Adding support for a new framework or language means writing a new adapter. No core changes.

## Language Bindings

Client bindings use per-language FFI tools rather than a single generator — each language has a mature, async-capable tool that provides the best developer experience:

| Language | FFI Tool | Async Bridging | Status |
|----------|----------|----------------|--------|
| Python | PyO3 + pyo3-async-runtimes | Rust async → Python awaitable | Implemented |
| Node.js | Napi-RS | Rust async → JS Promise | Planned |
| Ruby | Magnus + rb-sys | Manual reactor pattern (GVL release + channel) | Planned |

Server-side adapters for other languages (Rails, Django, Express) are written natively in each language, not via FFI. See [project-structure.md](project-structure.md) for details.

## Core Server API

The adapter-facing API has three functions:

### `OutpunchServer::new(config)`

Creates the server with a shared secret and request timeout.

```rust
let server = OutpunchServer::new(ServerConfig {
    secret: "shared-secret".into(),
    timeout: Duration::from_secs(25),
});
```

### `server.handle_request(request)`

Called by the adapter when an HTTP request hits `/tunnel/*path`. The adapter translates the framework's request into an `IncomingRequest` (service, method, path, query, headers, body — no `request_id`).

The core:
1. Looks up the service in the service map → if no client, returns 502 immediately
2. Generates a `request_id`
3. Parks a handler in the pending request map
4. Sends the `TunnelRequest` (with `request_id`) through the client's channel
5. Waits for a response (or timeout → 504)
6. Returns a `TunnelResponse` to the adapter

```rust
let response: TunnelResponse = server.handle_request(IncomingRequest {
    service: "my-service".into(),
    method: "POST".into(),
    path: "api/test".into(),
    query: HashMap::new(),
    headers: HashMap::new(),
    body: Some("...".into()),
}).await;
```

### `server.handle_connection(incoming_rx, outgoing_tx)`

Called by the adapter after WebSocket upgrade and bridge setup. Runs for the lifetime of the connection.

The core:
1. Reads the first message from `incoming_rx` — expects auth with token and service name
2. Validates the token → if invalid, sends error and returns
3. Registers the service → connection mapping in the service map
4. Loops: reads responses from `incoming_rx`, matches `request_id` to pending map, delivers responses
5. On disconnect: removes service mapping, fails any pending requests for this client with 502

```rust
server.handle_connection(incoming_rx, outgoing_tx).await;
```

## Client Architecture

The client is simpler — no adapters, no framework concerns. A standalone binary (phase 1) or embedded library (future).

### `OutpunchClient::new(config)`

```rust
let client = OutpunchClient::new(ClientConfig {
    server_url: "ws://localhost:3000/ws".into(),
    secret: "shared-secret".into(),
    service: "my-service".into(),
    forward_to: "http://localhost:8080".into(),
    reconnect_delay: Duration::from_secs(5),
});
```

### `client.run()`

Runs forever. Internally:

1. Connect to server via tokio-tungstenite
2. Send auth message with token and service name
3. Wait for `auth_ok` (or disconnect on rejection)
4. Loop: receive `TunnelRequest` from server, forward to local service via reqwest, send `TunnelResponse` back
5. On disconnect: wait `reconnect_delay`, go to step 1

```rust
client.run().await; // runs until process exits
```

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
