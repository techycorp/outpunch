# Outpunch — Testing Strategy

Stability and maturity are the primary goals. The test suite is structured in layers, from fast isolated unit tests to full-stack integration and failure mode coverage.

## Adapter Conformance Tests

A single test suite that defines the product contract. Written once, runs against every server framework adapter. Each test is a **technical product requirement** documented in clear English.

Examples:

- "A GET request to /tunnel/my-service/api/test is forwarded to the local service and the response is returned to the caller"
- "When no tunnel client is connected, requests return 502"
- "When the local service doesn't respond within the timeout, requests return 504"
- "A tunnel client must authenticate with a valid shared secret or the connection is rejected"
- "Request headers (except hop-by-hop) are forwarded to the local service"
- "Response headers from the local service are returned to the caller"
- "Multiple services are multiplexed on a single WebSocket connection"
- "Binary response bodies are base64-encoded and decoded correctly"

The test harness spins up a real server (using whichever adapter is under test), a real client, and a real local HTTP service. The tests don't know or care which adapter is running — they make HTTP requests and assert on responses.

When someone writes a new adapter (`outpunch-actix`, `outpunch-ruby-rack`, `outpunch-asgi`), they plug it into this same suite. If it passes, the adapter is conformant. If a test fails for one adapter but not another, it's caught a subtle framework difference.

This suite is the living spec. It is the most important layer.

## Core Unit Tests

Pure logic, no network. The core crate tested in isolation:

- Protocol serialization/deserialization (round-trip every message type)
- Pending request map lifecycle (register, complete, timeout, cleanup)
- Auth validation (valid token, invalid token, empty token, missing secret, constant-time comparison)
- Header filtering (hop-by-hop headers stripped, meaningful headers preserved)
- Message dispatch (request routed to correct service, unknown service rejected)

These are fast, deterministic, and thorough. They cover every branch in the core.

## Integration Tests

Real network, real WebSocket connections, real HTTP. Server and client as separate async tasks (or processes):

- Full round-trip: HTTP request → server → WS → client → local service → response back
- Multiple concurrent requests through the tunnel
- Multiple services multiplexed on one connection
- Binary/large body handling
- All HTTP methods (GET, POST, PUT, PATCH, DELETE)
- Query parameter forwarding

## Failure and Edge Case Tests

The happy path is easy. Failure modes are where production breaks:

- Client disconnects mid-request — pending request fails immediately with 502, not hang or wait for the timeout
- Client reconnects after disconnect — new requests should work immediately
- Server receives a response for an already-timed-out request — silently ignored, no panic
- Malformed WebSocket messages — invalid JSON, missing fields, wrong types
- Auth with wrong token, empty token, no auth message at all
- Client subscribes to a service then unsubscribes — requests to that service should 502
- Two clients register for the same service — the newer one takes over and the older connection is closed (see [future.md](future.md))
- A caller is cancelled while its request is pending — the pending entry is removed (F1 in [spec/FINDINGS.md](../spec/FINDINGS.md))
- Server shuts down cleanly while requests are in-flight
- WebSocket connection drops and reconnects under load

## Property and Fuzz Tests

For a protocol library where stability matters:

- Fuzz the JSON message parser with arbitrary bytes — should never panic
- Property test: any valid `TunnelRequest` survives serialize → deserialize round-trip identically
- Property test: any valid `TunnelResponse` survives serialize → deserialize round-trip identically

## Formal Models (TLA+)

The core's concurrency (auth, connection replacement, pending requests, timeouts, cleanup), the axum
adapter's connection path and the Ruby binding's interrupt handling are modeled in TLA+ in
[spec/](../spec/README.md) and checked with TLC: `just tla-all`. Requirements are in
[spec/REQUIREMENTS.md](../spec/REQUIREMENTS.md); results in [spec/FINDINGS.md](../spec/FINDINGS.md); what the
runs do, in [spec/HOW-IT-WORKS.md](../spec/HOW-IT-WORKS.md).

- A model checks the design, not the Rust. When it finds a violation, a unit test reproduces it in the
  code; open findings have `#[ignore]`d reproductions that fail until fixed
  (`cargo test -p outpunch -- --ignored`, `cargo test -p outpunch-axum -- --ignored`).
- `just tla-traces` checks that executions of the core's unit tests are behaviors of the core model
  (trace validation, using the `trace` feature). Run it after changing `server.rs` or `Tunnel.tla`.
- `just tla-mutants` checks that the core model's properties catch a list of known bugs.
- When changing modeled code, follow "Keeping models in step with the code" in
  [spec/README.md](../spec/README.md): the models do not update themselves.

## Structure

```
crates/
  outpunch/
    tests/              # core unit tests
  outpunch-axum/
    tests/              # axum-specific unit tests (if any)

tests/
  conformance/          # adapter conformance suite (product requirements)
  integration/          # full-stack integration tests
  failure/              # edge cases and failure modes
```
