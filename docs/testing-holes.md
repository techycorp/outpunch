# Outpunch — Testing Coverage Gaps

Known gaps in automated test coverage and why they exist.

## Adapter Bridge Loop (3 lines)

`crates/outpunch-axum/src/lib.rs`

These are defensive `break` statements in the WebSocket bridge loop that fire during connection teardown races. They're correct but hard to trigger deterministically in tests.

- **Line 115** — `incoming_tx.send` fails. The core dropped its receiver (i.e., `handle_connection` returned) while the read bridge is still pulling messages from the WebSocket. Only happens during shutdown when the core finishes before the bridge loop does.

- **Line 119** — `_ => {}` catch-all for WS frame types that aren't Text or Close (e.g., Ping, Pong). These are handled at the WebSocket protocol level by tungstenite before reaching application code, so this branch effectively never fires in practice.

- **Line 128** — `ws_sink.send` fails. The client's TCP connection dropped mid-write. Requires the client to disconnect at exactly the moment the server is writing a message — a network-level race condition.

## Client Binary (thin CLI wrapper)

`crates/outpunch-client/src/main.rs` — 0% coverage

The client was refactored into a library (`lib.rs`, 92% coverage) + thin CLI wrapper (`main.rs`). The wrapper is just clap arg parsing → `ClientConfig` → `outpunch_client::run()`. Tarpaulin can't instrument the binary since it runs as a separate process, but the actual logic is tested through the library.
