# outpunch

Reverse WebSocket tunnel proxy. One Rust core, any framework, any language.

```
HTTP request → public server → WebSocket tunnel → private service
```

The core is a framework-agnostic Rust library — no web framework or WebSocket library dependency. Server adapters and client bindings are thin wrappers, not reimplementations.

**Server adapters** — axum, Vite, Express, Fastify, Rack, Rails

**Client bindings** — Rust, Python, Node.js, Ruby

> **Windows:** the Ruby gems are not supported on Windows yet. Other packages are tested on Windows.

> **Ruby gem names:** install `outpunch-ruby`, `outpunch-ruby-rack` and `outpunch-ruby-rails`. Due to a clerical error we lost access to the RubyGems account that owns the original `outpunch`, `outpunch-rack` and `outpunch-rails` gems. We created those gems, and they are now orphaned at 0.1.x and will not get updates. If you have them installed, remove them before installing the new names, since both provide `require "outpunch"`. If we recover the account, we'll move back to the original names. `require` paths and module names are unchanged.

## Install

| Language | Package | Install |
|---|---|---|
| Rust | server adapter, client | `cargo add outpunch-axum` / `cargo add outpunch-client` |
| Python | client | `pip install outpunch` |
| Node.js | server (Vite, Node http) | `npm install outpunch` |
| Ruby | Rack / Rails server adapter | `bundle add outpunch-ruby-rack` / `bundle add outpunch-ruby-rails` |

Every package shares one version. `outpunch-ruby` ships precompiled for `x86_64-linux` (Ruby 3.1 to
4.0). On any other platform it compiles a Rust extension on install, so it needs a Rust toolchain
(`cargo`).

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

Releasing: see [docs/releasing.md](docs/releasing.md).

## License

MIT
