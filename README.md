# outpunch

Reverse WebSocket tunnel proxy — expose private services through a public server without opening inbound ports.

A client on the private network connects **outbound** via WebSocket to the server. The server relays HTTP requests through the tunnel and returns responses. No inbound ports, no NAT config, no VPN.

```
                    internet
                       │
    ┌──────────────────┼──────────────────┐
    │  Public Server   │                  │
    │  (axum, Vite,    │                  │
    │   Express, etc.) │                  │
    │                  │                  │
    │  /tunnel/svc/* ──┤                  │
    │                  │  WebSocket       │
    │  /ws ────────────┤  (outbound       │
    │                  │   from client)   │
    └──────────────────┼──────────────────┘
                       │
    ┌──────────────────┼──────────────────┐
    │  Private Network │                  │
    │                  │                  │
    │  outpunch-client ┤──► local service │
    │                  │    (port 8080)   │
    └──────────────────┴──────────────────┘
```

## Quick Start

### Server (Rust, embedded in axum)

```rust
let server = OutpunchServer::new(ServerConfig {
    secret: "shared-secret".into(),
    ..Default::default()
});
let app = outpunch_axum::router(server);
```

### Server (Vite plugin)

```ts
import { outpunch } from 'outpunch/vite';

export default defineConfig({
  plugins: [outpunch({ secret: 'shared-secret' })],
});
```

### Client (Rust binary)

```sh
outpunch-client \
  --server-url ws://your-server.com/ws \
  --secret shared-secret \
  --service my-api \
  --forward-to http://localhost:8080
```

### Client (Python)

```sh
pip install outpunch
```

```python
from outpunch import ClientConfig, run

run(ClientConfig(
    server_url="ws://your-server.com/ws",
    secret="shared-secret",
    service="my-api",
    forward_to="http://localhost:8080",
))
```

## Architecture

The core is a framework-agnostic Rust library. All tunnel logic — protocol parsing, request/response coordination, authentication — lives in the core using plain types and message channels. No web framework or WebSocket library dependency.

**Server adapters** are thin layers (~50 lines) that translate between a framework's types and the core:

| Adapter | Status |
|---------|--------|
| Rust/axum | Shipped |
| Node.js/Vite | Shipped |
| Node.js/Express | Planned |

**Client bindings** wrap the Rust client library via per-language FFI tools:

| Language | FFI Tool | Status |
|----------|----------|--------|
| Python | PyO3 | Shipped |
| Node.js | Napi-RS | Shipped |
| Ruby | Magnus | Planned |

## Development

Requires: Rust, Python 3.10+ (with uv), Node.js 18+ (with npm).

```sh
just test-all     # run all test suites (Rust + Python + Node.js)
just coverage     # Rust test coverage
just lint         # rustfmt + clippy
```

## License

MIT
