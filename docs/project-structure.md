# Project Structure

## Rust Crates

```
crates/
  outpunch/              # Core library — protocol types, request matching, auth, connection handling
  outpunch-axum/         # Axum server adapter — thin bridge between axum HTTP/WS and core
  outpunch-client/       # Client library — WebSocket tunnel client with HTTP forwarding
```

All three are workspace members in the root `Cargo.toml`. The core has zero framework dependencies. The adapter and client depend on the core via path dependency.

## Language Bindings

Each language binding lives in `bindings/<language>/` with its own package manifest, build config, and a thin Rust FFI crate that wraps `outpunch-client`.

```
bindings/
  python/
    pyproject.toml           # Python package config (built by Maturin)
    src/outpunch_client/     # Python package
      __init__.py
    rust/
      Cargo.toml             # PyO3 crate, depends on outpunch-client
      src/lib.rs             # PyO3 glue

  node/
    package.json             # npm package config
    index.js
    index.d.ts               # Auto-generated TypeScript types
    rust/
      Cargo.toml             # Napi-RS crate, depends on outpunch-client
      src/lib.rs             # Napi-RS glue

  ruby/
    outpunch_client.gemspec  # Gem config
    lib/outpunch_client/     # Ruby package
    ext/outpunch_client/
      Cargo.toml             # Magnus crate, depends on outpunch-client
      src/lib.rs             # Magnus glue
```

### How it works

Each binding has two layers:

1. **Rust FFI crate** — uses the language-specific FFI tool (PyO3, Napi-RS, Magnus) to expose `outpunch-client` types and functions to the target language. These crates are workspace members so `cargo build` covers them and they use path dependencies on `outpunch-client`.

2. **Language-native package** — the files users actually install (`pyproject.toml`, `package.json`, `.gemspec`). The language's build tooling compiles the FFI crate into a native extension as part of the package build.

### FFI tools per language

| Language | FFI Tool | Async Bridging | Package Tool |
|----------|----------|----------------|--------------|
| Python | PyO3 + pyo3-async-runtimes | Rust async fn → Python awaitable | Maturin → PyPI |
| Node.js | Napi-RS | Rust async fn → JS Promise | @napi-rs/cli → npm |
| Ruby | Magnus + rb-sys | Manual reactor pattern (GVL release + channel) | rake-compiler → RubyGems |

### Build and release

Each language has its own CI workflow, test suite (in that language), and release process. They are independent — a Python release does not require changes to Node or Ruby.

The Rust workspace builds everything together (`cargo build`, `cargo test`), but language-specific builds are handled by their respective toolchains (Maturin, @napi-rs/cli, rake-compiler).

## Server Adapters vs Client Bindings

The language bindings wrap the **client** — it's framework-agnostic, so a single Rust library works for every user regardless of their web framework.

**Server adapters** are different. They must integrate with the user's web framework (Rails, Django, Express, Spring), so each is written natively in that language. The adapter translates framework HTTP/WS types into core protocol types. The core logic (pending request map, auth, protocol) can still be called via FFI from the native adapter.

```
Server side:  framework-native adapter → core (via FFI or reimplemented)
Client side:  language binding (via FFI) → outpunch-client (Rust)
```

## Documentation

```
docs/
  product.md             # Product overview, protocol spec, auth model
  architecture.md        # Crate design, core principles, adapter pattern
  project-structure.md   # This file
  testing.md             # Testing strategy
  testing-holes.md       # Known coverage gaps
  phase1.md              # Phase 1 implementation plan
  future.md              # Future work and open questions
  glossary.md            # Term definitions
  tunnel-proxy.md        # Original design doc
```
