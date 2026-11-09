# Outpunch Core: Requirements for the TLA+ Spec

Scope: the server core only, `crates/outpunch/src/server.rs` and `crates/outpunch/src/protocol.rs`.
Clients are modeled as arbitrary parties (they may authenticate or not, respond, not respond, respond
late, respond with any `request_id`, or disconnect), not as our well-behaved client.

Each requirement cites where it comes from. "Decided" means settled in review where the docs were silent
or stale. Line numbers are as of commit `2509723`.

## Auth

1. A connection receives no requests until it authenticates with the shared secret.
   Source: `docs/product.md:41`, `docs/architecture.md:187-189`.
2. A server with an empty secret rejects every client.
   Source: `CHANGELOG.md:151-152`, commit `5412e18`.
3. The first message must be `auth`; anything else is rejected.
   Source: `docs/architecture.md:187`; test `connection_rejects_non_auth_first_message`.
4. A connection that does not authenticate within `auth_timeout` is closed.
   Source: `CHANGELOG.md:165-166`.
5. On auth failure the server sends `auth_error` and ends the connection: `run()` returns and the
   adapter closes the socket (axum after a flush of up to 100 ms). A connection whose channel closes
   during auth gets no `auth_error`.
   Source: tests `auth_invalid_token_rejects`, `connection_rejects_non_auth_first_message`,
   `auth_rejects_empty_token_when_secret_unset`; the client prints the `auth_error` reason
   (`crates/outpunch-client/src/lib.rs`, `authenticate`). Decided: `docs/product.md:97` and
   `docs/tunnel-proxy.md:268` ("server closes the connection") are stale, not contradicted.
6. One service per connection.
   Source: code. Multiple services per connection was the plan (`docs/product.md:58,68,95`,
   `docs/future.md` "Multiple Services Per Client") and is backlog; see [notes/multi-client.md](notes/multi-client.md).

## Routing

7. A request for a service with no connected client gets 502 immediately.
   Source: `docs/architecture.md:164`, `docs/tunnel-proxy.md:192`.
8. Each request goes to the client currently connected for its service.
   Source: `docs/product.md:85`.
9. When a second client authenticates for a service, it takes over and the older connection is closed
   (newest wins).
   Source: `CHANGELOG.md:175-176`, `docs/future.md:17`, commit `5412e18`.
10. A stale disconnect never removes a newer connection.
    Source: `docs/architecture.md:35,75`, `docs/tunnel-proxy.md:95,306`, `CHANGELOG.md:156-157`.

## Request lifecycle

11. Every request gets a unique `request_id`.
    Source: `docs/architecture.md:165`, `docs/tunnel-proxy.md:54`.
12. A response is delivered to the caller waiting on its `request_id`, and to no other caller.
    Source: `docs/architecture.md:29-33`.
13. Only the connection a request was sent to can answer it.
    Source: `CHANGELOG.md:167`, commit `5412e18`.
14. Every caller gets exactly one outcome (response, 502, or 504) and never hangs.
    Source: `docs/architecture.md:35`, `docs/product.md:44`.
15. No response within the configured timeout gives 504.
    Source: `docs/architecture.md:168`, `docs/tunnel-proxy.md:193`.
16. The timeout covers sending the request, so a client that stops reading cannot block past it.
    Source: `CHANGELOG.md:160-161`.
17. When a client disconnects mid-request, its in-flight requests fail immediately with 502.
    Source: `CHANGELOG.md:158-159`, `docs/architecture.md:191`; test
    `disconnect_mid_request_fails_fast_with_502`. Decided: `docs/tunnel-proxy.md:196` and
    `docs/testing.md:53` ("times out") are stale. No consumer depends on a 504 here: the Ruby binding maps
    it to a generic `TunnelError` (Rails answers 502); Node and Python receive the 502 response.
18. A response with an unknown or expired `request_id` is ignored.
    Source: `docs/tunnel-proxy.md:98,198`.
19. Every outcome removes the request from the pending map (no leaks).
    Source: `docs/architecture.md:33,35`, `docs/tunnel-proxy.md:97`.
20. A stalled client does not block requests to other services.
    Source: `CHANGELOG.md:160`; test `stalled_service_does_not_block_other_services`.
21. After a client reconnects, new requests work immediately.
    Source: `docs/testing.md:54`.
22. A malformed message after auth is ignored and the connection stays up.
    Source: test `connection_ignores_malformed_messages`. Decided: intended.
23. Requests already sent to a connection that is then replaced get that connection's response if it was
    already received, and otherwise fail with 502 when the old connection is cleaned up. They are never
    resent to the new client.
    Source: `docs/future.md:17` ("fails its in-flight requests with 502. This favors fast reconnects").
    Resending is excluded because the request may already have been applied (RFC 9110 §9.2.2).

## Added after the first model (review and adapter work)

24. `run()` cleans up on every exit path, including its future being dropped: the service is
    unregistered and the connection's pending requests fail with 502. Source: decided, the mirror of R19
    for connections. Holds since the F3 fix ([FINDINGS.md](FINDINGS.md)).
25. If the drain task dies (the `on_message` callback panics), the service does not stay routed to the
    connection. Source: decided. Holds since the F4 fix.
26. The adapter queues outgoing messages only for requests still in progress, so memory per connection is
    bounded by the requests in flight. Source: decided. Still violated: since the F7 fix, memory under
    overload is about arrival rate × request timeout instead (F7, R29).
27. A request is not written to the client's socket after its caller has received an answer (e.g. 504).
    Source: decided; a caller that got 504 may retry, and a late delivery can run the request twice.
    Holds in the axum, Ruby and Node adapters since the F5 fix, except for the one request already being written when its
    caller is answered. A request written before its timeout can still be read
    and run after it; no adapter can prevent that, only the protocol (e.g. a deadline the client
    enforces).
28. Once a connection is over, its socket and adapter tasks end, even if the peer never reads again.
    Source: decided. Holds in the axum adapter since the F6 fix.
29. A client that stays connected but stops reading is disconnected once one write to it has waited
    longer than the request timeout, so its service is unregistered. Source: decided (F7). Holds in the
    axum, Rack/Rails and Node adapters.

## API contract (not modeled)

- `on_message` must be called before `run()`; a callback registered later is never used, so every
  outgoing message, including `auth_ok`, is dropped.
- `run()` may be called once; a second call panics.
- `push_message` before `run()` is allowed and buffered; it waits once 64 messages are buffered.
- Ruby binding: `next_outgoing` is meant for one thread per connection; see F2.

## Known consequence of current behavior (to show in the model)

Two live clients configured for the same service displace each other: each is closed, reconnects after
its retry delay (client default 5s, `crates/outpunch-client/src/lib.rs:30`), and closes the other. Requests
in flight at each swap get 502. No request gets a wrong answer. Source: `docs/future.md:17,73`. Shown by `spec/Flapping.cfg`.

## Not modeled in TLA+

Real requirements that are not about behavior over time; covered by tests, fuzzing, or other tools:

- Constant-time secret comparison (`docs/architecture.md:76`): a timing side channel.
- JSON message shapes and parsing (`crates/outpunch/src/protocol.rs`): property tests and fuzzing.
- `max_body_size` limits.
- The adapters (`crates/outpunch-axum`, `adapters/ruby`, `bindings/*`) and the client crate.
