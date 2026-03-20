# Outpunch — Testing Coverage Gaps

Known gaps in automated test coverage and why they exist.

## Adapter Bridge Loop (3 lines)

`crates/outpunch-axum/src/lib.rs`

These are defensive `break` statements in the WebSocket bridge loop that fire during connection teardown races. They're correct but hard to trigger deterministically in tests.

- **Line 115** — `incoming_tx.send` fails. The core dropped its receiver (i.e., `handle_connection` returned) while the read bridge is still pulling messages from the WebSocket. Only happens during shutdown when the core finishes before the bridge loop does.

- **Line 119** — `_ => {}` catch-all for WS frame types that aren't Text or Close (e.g., Ping, Pong). These are handled at the WebSocket protocol level by tungstenite before reaching application code, so this branch effectively never fires in practice.

- **Line 128** — `ws_sink.send` fails. The client's TCP connection dropped mid-write. Requires the client to disconnect at exactly the moment the server is writing a message — a network-level race condition.

## Client Binary (entire file)

`crates/outpunch-client/src/main.rs` — 0% coverage

Tarpaulin can't instrument the client binary because it runs as a separate process. The client's logic is exercised indirectly by the integration tests (which simulate a client via WebSocket), but the actual binary code paths — CLI parsing, reconnection loop, reqwest HTTP forwarding — are not counted.

Options to improve this in the future:
- Extract the client logic into a library crate with its own unit tests, leaving `main.rs` as a thin CLI wrapper
- Use an end-to-end test that spawns the actual binary and measures coverage via `LLVM_PROFILE_FILE`
