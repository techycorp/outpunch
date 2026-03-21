# outpunch

Reverse WebSocket tunnel proxy. One Rust core, any framework, any language.

```
HTTP request → public server → WebSocket tunnel → private service
```

The core is a framework-agnostic Rust library — no web framework or WebSocket library dependency. Server adapters and client bindings are thin wrappers, not reimplementations.

**Server adapters** — embed the tunnel in your existing app:

| Framework | Integration | Status |
|-----------|------------|--------|
| Rust/axum | `outpunch_axum::router(server)` | Shipped |
| Vite | `plugins: [outpunch({ secret })]` | Shipped |
| Express | `app.use(outpunchMiddleware({ secret }))` | Planned |
| Fastify | | Planned |

**Client bindings** — connect from any language:

| Language | Install | Status |
|----------|---------|--------|
| Rust | `cargo add outpunch-client` | Shipped |
| Python | `pip install outpunch` | Shipped |
| Node.js | `npm install outpunch` | Shipped |
| Ruby | `gem install outpunch` | Planned |

## Quick Start

### Server (Vite)

```ts
import { outpunch } from 'outpunch/vite';

export default defineConfig({
  plugins: [outpunch({ secret: 'shared-secret' })],
});
```

### Server (Rust/axum)

```rust
let server = OutpunchServer::new(ServerConfig {
    secret: "shared-secret".into(),
    ..Default::default()
});
let app = outpunch_axum::router(server);
```

### Client (Python)

```python
from outpunch import ClientConfig, run

run(ClientConfig(
    server_url="ws://your-server.com/ws",
    secret="shared-secret",
    service="my-api",
    forward_to="http://localhost:8080",
))
```

### Client (Rust)

```sh
outpunch-client \
  --server-url ws://your-server.com/ws \
  --secret shared-secret \
  --service my-api \
  --forward-to http://localhost:8080
```

## Development

Requires: Rust, Python 3.10+ (with uv), Node.js 18+ (with npm).

```sh
just test-all     # run all test suites (Rust + Python + Node.js)
just coverage     # Rust test coverage
just lint         # rustfmt + clippy
```

## License

MIT
