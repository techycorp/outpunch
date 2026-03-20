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

Each language binding lives in `bindings/<language>/` with its own package manifest, build config, and a Rust FFI crate that wraps `outpunch-client`. The FFI crate and package config live at the binding root (following uv/maturin conventions), with Rust source in `src/` alongside the Python package directory.

```
bindings/
  python/                      # Python binding (implemented)
    Cargo.toml                 # PyO3 FFI crate (workspace member)
    pyproject.toml             # uv + Maturin config
    justfile                   # build/dev/test commands
    src/
      lib.rs                   # PyO3 glue code
      lib_tests.rs             # Rust-side unit tests
      outpunch/                # Python package
        __init__.py            # Re-exports from _core
        _core.pyi              # Type stubs for editors
        py.typed               # PEP 561 marker
    tests/
      test_config.py           # Python unit tests
      test_run.py              # Python integration tests

  node/                        # Node.js binding (planned)
    package.json
    Cargo.toml                 # Napi-RS crate
    src/lib.rs

  ruby/                        # Ruby binding (planned)
    outpunch.gemspec
    Cargo.toml                 # Magnus crate
    ext/outpunch/src/lib.rs
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
  phases.md              # Implementation phases
  future.md              # Future work and open questions
  glossary.md            # Term definitions
  tunnel-proxy.md        # Original design doc
```
