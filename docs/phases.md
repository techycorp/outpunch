# Outpunch — Phases

## Table of Contents

- [Phase 1: Embedded Mode](#phase-1-embedded-mode-complete) — Core tunnel, axum adapter, client binary
- [Phase 2: Python Bindings](#phase-2-python-bindings-complete) — PyO3 + Maturin + uv

---

## Phase 1: Embedded Mode (Complete)

Proves the core tunnel works end-to-end in embedded mode. One server, one client, one service, minimal scope. 77 Rust tests, 94% coverage.

### Goal

A Rust library and client binary that can:

1. Server (embedded in a Rust app via axum adapter): accept a WebSocket connection from a client, accept HTTP requests on `/tunnel/:service/*path`, relay requests through the tunnel, return responses
2. Client (standalone binary): connect outbound via WebSocket, authenticate, receive tunnel requests, forward them to a local HTTP service, send responses back
3. Round-trip works: `curl /tunnel/my-service/api/test` → through the tunnel → local service → response back to curl

### Scope

**In scope:**

- **Core crate** (`outpunch`): protocol types, message serialization, pending request map, connection state, auth validation, header filtering, channel-based WebSocket interface
- **Axum adapter** (`outpunch-axum`): tunnel HTTP endpoint, WebSocket upgrade, bridge loop
- **Client binary** (`outpunch-client`): WebSocket connection, auth handshake, HTTP forwarding via reqwest, reconnection on disconnect
- **Single service**: one client registers for one service name
- **Shared secret auth**: constant-time comparison, reject on failure
- **Timeout handling**: configurable request timeout, 504 on expiry, cleanup
- **Base64 body encoding**: keep it simple, match the existing protocol
- **Core unit tests**: protocol round-trip, pending request lifecycle, auth, header filtering
- **Integration test**: full round-trip through real network (server + client + local HTTP service)

**Out of scope** (see [future.md](future.md)):

Standalone server binary, multiple clients per service, multiple services per client, language bindings, non-axum adapters, binary WS frames, TLS/mTLS, service authorization, hot-reload, Helm charts.

### Crate Structure

```
Cargo.toml              (workspace)
crates/
  outpunch/             # core library
  outpunch-axum/        # axum adapter
  outpunch-client/      # client library + binary
```

### Core API

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

### Client Binary

```
outpunch-client \
  --server-url ws://localhost:3000/ws \
  --secret "shared-secret" \
  --service my-service \
  --forward-to http://localhost:8080
```

Single service, single connection. Reconnects on disconnect with configurable delay.

---

## Phase 2: Python Bindings (Complete)

Python client bindings via PyO3, built with Maturin and managed with uv. 3 Rust tests + 5 Python tests.

### Goal

`pip install outpunch` gives Python users a native tunnel client:

```python
from outpunch import ClientConfig, run

config = ClientConfig(
    server_url="wss://tunnel.example.com/ws",
    secret="my-secret",
    service="my-app",
    forward_to="http://localhost:8080",
)
run(config)
```

### What shipped

- **PyO3 FFI crate** (`bindings/python/`): wraps `outpunch-client` types and functions
- **`ClientConfig`** exposed as a Python class with `get`/`set` on all fields
- **`run(config)`** — blocking call (releases GIL), reconnects forever
- **`run_connection(config)`** — async, returns Python awaitable via `pyo3-async-runtimes`
- **Duration fields as `f64` seconds** — `reconnect_delay=5.0` not `timedelta(seconds=5)`
- **Type stubs** (`_core.pyi`) for editor support
- **Separate justfile** at `bindings/python/justfile`, delegated from root

### Key decisions

- **Per-language FFI tools over UniFFI**: UniFFI's Node.js support is experimental and Ruby support is deprecated. PyO3 for Python, Napi-RS for Node, Magnus for Ruby — each is the most mature option for its language.
- **Blocking `run()` as primary API**: the tunnel client runs forever, so blocking is the natural interface. Async `run_connection()` available for users who want connection lifecycle control.
- **Tokio runtime managed by `pyo3-async-runtimes`**: lazy global runtime, no manual initialization needed.
- **Package name `outpunch`**: `pip install outpunch`. Future server-side packages are `outpunch-django`, `outpunch-fastapi`, etc.
