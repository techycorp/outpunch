# Outpunch — Phase 1: Embedded Mode

Phase 1 proves the core tunnel works end-to-end in embedded mode. One server, one client, one service, minimal scope.

## Goal

A Rust library and client binary that can:

1. Server (embedded in a Rust app via axum adapter): accept a WebSocket connection from a client, accept HTTP requests on `/tunnel/:service/*path`, relay requests through the tunnel, return responses
2. Client (standalone binary): connect outbound via WebSocket, authenticate, receive tunnel requests, forward them to a local HTTP service, send responses back
3. Round-trip works: `curl /tunnel/my-service/api/test` → through the tunnel → local service → response back to curl

## Scope

### In scope

- **Core crate** (`outpunch`): protocol types, message serialization, pending request map, connection state, auth validation, header filtering, channel-based WebSocket interface
- **Axum adapter** (`outpunch-axum`): tunnel HTTP endpoint, WebSocket upgrade, bridge loop
- **Client binary** (`outpunch-client`): WebSocket connection, auth handshake, HTTP forwarding via reqwest, reconnection on disconnect
- **Single service**: one client registers for one service name
- **Shared secret auth**: constant-time comparison, reject on failure
- **Timeout handling**: configurable request timeout, 504 on expiry, cleanup
- **Base64 body encoding**: keep it simple, match the existing protocol
- **Core unit tests**: protocol round-trip, pending request lifecycle, auth, header filtering
- **Integration test**: full round-trip through real network (server + client + local HTTP service)

### Out of scope (see future.md)

- Standalone server binary
- Multiple clients per service / load balancing
- Multiple services per client
- Language bindings (Python, Ruby, Node.js)
- Non-axum adapters
- Binary WebSocket frames (alternative to base64)
- TLS/mTLS
- Service authorization (beyond shared secret)
- Hot-reload of service configuration
- Helm charts, Docker images

## Crate Structure (Phase 1)

```
Cargo.toml              (workspace)
crates/
  outpunch/             # core library
  outpunch-axum/        # axum adapter
  outpunch-client/      # client binary
```

No standalone server binary in phase 1. The server is always embedded via the axum adapter.

## Core API (Phase 1)

```rust
let server = OutpunchServer::new(ServerConfig {
    secret: "shared-secret".into(),
    timeout: Duration::from_secs(25),
});

// Adapter calls when HTTP request hits /tunnel/*
let response = server.handle_request(request).await;

// Adapter calls after WS upgrade + bridge setup
server.handle_connection(incoming_rx, outgoing_tx).await;
```

`handle_request` takes an `IncomingRequest` (service, method, path, query, headers, body — no request_id, the core generates that). Returns a `TunnelResponse`.

`handle_connection` runs for the lifetime of the WebSocket connection. Reads auth + responses from `incoming_rx`, writes auth_ok + requests to `outgoing_tx`.

## Client Binary (Phase 1)

```
outpunch-client \
  --server-url ws://localhost:3000/ws \
  --secret "shared-secret" \
  --service my-service \
  --forward-to http://localhost:8080
```

Single service, single connection. Reconnects on disconnect with configurable delay.

## What "Done" Looks Like

You can run this and it works:

```bash
# Terminal 1: your app with outpunch server embedded
cargo run --bin your-app  # axum app with outpunch routes mounted

# Terminal 2: outpunch client forwarding to a local service
cargo run --bin outpunch-client -- --server-url ws://localhost:3000/ws ...

# Terminal 3: a local service (anything that speaks HTTP)
python -m http.server 8080

# Terminal 4: hit the tunnel
curl http://localhost:3000/tunnel/my-service/
# → response from the python HTTP server
```
