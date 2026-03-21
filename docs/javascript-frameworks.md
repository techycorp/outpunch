# JavaScript Server Adapters

## Strategy

The JavaScript server adapter wraps the Rust core (`outpunch` crate) via Napi-RS, keeping a single implementation of all tunnel logic. Framework-specific adapters are thin wrappers that delegate to a shared Node.js core.

## Architecture

```
outpunch (npm package)
├── Napi-RS bindings     →  OutpunchServer, handle_request, handle_connection
├── http.Server adapter  →  /tunnel/* route handling, /ws upgrade, WS bridge
├── vite plugin          →  configureServer hook → core
├── express middleware    →  app.use() → core
├── fastify plugin       →  fastify.register() → core
└── hono middleware       →  app.use() → core
```

### Layer 1: Napi-RS Bindings (Rust → JS)

Wraps the Rust `outpunch` crate, exposing `OutpunchServer` and its methods to JavaScript.

**HTTP requests** (`handle_request`): straightforward async call. JS passes plain types (service, method, path, query, headers, body), gets back a Promise that resolves with the response (status, headers, body). Maps directly to a Napi-RS async function.

**WebSocket connections**: the core exposes a `Connection` object with a simple push/callback interface. The Napi-RS binding wraps two methods:

- `connection.push_message(text)` — JS calls this when a WS message arrives (Napi-RS method call)
- `connection.on_message(callback)` — Rust calls this callback when it has an outgoing message (`ThreadsafeFunction`)
- `connection.run()` — starts the connection lifecycle on the tokio runtime, returns a Promise that resolves when the connection ends

The `Connection` owns the channels internally — the Napi-RS layer doesn't manage channel creation or lifecycle. This keeps the binding thin.

### Layer 2: Node.js http.Server Adapter

Works at the raw `http.Server` level — handling `request` events for tunnel HTTP and `upgrade` events for WebSocket connections. This is the broadest abstraction because every Node.js framework sits on top of `http.Server`.

Responsibilities:
- Route matching: identify `/tunnel/:service/*path` requests and `/ws` upgrade requests
- HTTP requests: extract method, path, query, headers, body → call `handle_request` → write response
- WebSocket upgrade: accept the upgrade, call `connection.push_message()` on incoming, send `on_message` callbacks to WS
- Everything else: pass through to the framework's normal handling

### Layer 3: Framework Wrappers

Each wrapper is ~10-20 lines that hooks into the framework's extension point and delegates to the http.Server adapter.

**Vite**: `configureServer` plugin hook provides access to the dev server's Connect middleware stack and the underlying `http.Server` for WebSocket upgrades.

**Express**: standard `app.use()` middleware for HTTP, plus `server.on('upgrade')` for WebSocket.

**Fastify**: `fastify.register()` plugin with route handlers and WebSocket support via `@fastify/websocket`.

**Hono**: `app.use()` middleware. Hono on Node.js runs on the standard `http.Server`.

## Why Napi-RS, Not Pure JS

The Rust core is ~400 lines but contains real concurrency logic: pending request map with atomic operations, timeout handling, channel-based WebSocket coordination, constant-time auth. Reimplementing this in JS means a second implementation to maintain and test, with its own concurrency bugs.

Napi-RS keeps one implementation. The cost is cross-platform CI builds (automated by `@napi-rs/cli`) and the `ThreadsafeFunction` bridge for WebSocket connections. Users see a normal npm package — no build tools, no Rust toolchain, no node-gyp.

## Platform Builds

Napi-RS publishes precompiled binaries as platform-specific npm packages (e.g., `@outpunch/darwin-arm64`, `@outpunch/linux-x64-gnu`). The main `outpunch` package lists these as `optionalDependencies` — npm automatically installs only the one matching the user's OS and architecture.

Cross-compilation for ~15 targets is handled by GitHub Actions workflows scaffolded by `@napi-rs/cli`. This is the same approach used by SWC, Rspack, Rolldown, and Lightning CSS.

## Compatibility

| Runtime | Support | Notes |
|---------|---------|-------|
| Node.js | Full | Primary target, Node-API is the native interface |
| Bun | ~95% | Most Napi-RS packages work, edge cases remain |
| Deno | Partial | `deno_napi` compat layer, not fully reliable |

Napi-RS v3 supports WebAssembly compilation as a universal fallback for runtimes with incomplete Node-API support.

## Package Structure

```
bindings/node/
  Cargo.toml              # Napi-RS crate (workspace member)
  package.json            # npm package config
  src/
    lib.rs                # Napi-RS bindings for OutpunchServer
    lib_tests.rs          # Rust-side tests
  js/
    server.ts             # http.Server adapter
    vite.ts               # Vite plugin
    express.ts            # Express middleware
    index.ts              # Public API
  tests/
    server.test.ts        # Node.js adapter tests
    vite.test.ts          # Vite plugin tests
```
