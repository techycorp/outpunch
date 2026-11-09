# Findings

| ID | Where | Status | Found by | Rust reproduction |
|---|---|---|---|---|
| F1 | core: cancelled request leaks its pending entry | fixed | TLC (`NoLeak`) | `cancelled_request_*` (pass) |
| F2 | Ruby binding: `next_outgoing` ignores exceptions while waiting for the receiver lock | fixed | TLC (`RubyGvl.tla`) | `interrupt_spec.rb` (pass) |
| F3 | core: a dropped `run()` future skips cleanup | fixed | review, then TLC (`RunDrop*.cfg`) | `dropped_run_unregisters_and_fails_pending` (pass) |
| F4 | core: a dead drain task leaves the service registered | fixed | review, then TLC (`DrainDeath.cfg`) | `dead_callback_unregisters_service` (pass) |
| F5 | adapters: timed-out requests stay queued and are written to the socket late | fixed | TLC (`AxumLate.cfg`) | axum `timed_out_requests_are_not_delivered_later`, Ruby `server_spec.rb`, Node `server.test.ts` (all pass) |
| F6 | axum: a never-reading peer keeps the write task and socket alive | fixed | TLC (`AxumStalled.cfg`) | `socket_of_a_finished_connection_closes_even_if_the_client_never_reads` (pass) |
| F7 | adapters: a connected client that stops reading makes server memory grow without limit | fixed (bounded by the request timeout) | TLC (`AxumStale.cfg`, `AxumStuck.cfg`) | axum `client_that_stops_reading_is_disconnected`, Rack `connection_spec.rb`, Node `server.test.ts` (all pass) |
| F8 | client and server: a half-open connection goes unnoticed for about 15 minutes | fixed | production outage 2026-10-24 | client `run_connection_gives_up_on_a_server_that_stops_answering`; core `silent_client_is_evicted_after_the_idle_timeout` (pass) |

Each fix was checked the same way: the model's expected failure became a pass, a mutation in
`spec/mutations.txt` that undoes the fix is caught, and the reproduction fails without the fix and passes
with it. Library behavior each fix relies on has a vendor test (`vendor-claims.toml`).

## F1: a cancelled request leaves its pending entry until the connection ends

Status: **fixed.** Found by TLC (property `NoLeak`, requirement R19), reproduced in Rust by two core
unit tests that failed on the unfixed code, then fixed in code and model. The tests and both TLA+ properties
now pass.

### What happens

`dispatch()` inserts the pending entry under the lock, then awaits the send and the response. The entry is
removed by the relay loop when the client answers, by `cleanup_connection` when the connection ends, or by
`dispatch()` itself after an error (`if result.is_err() { ... pending.remove(...) }`).

If the dispatch future is dropped while it awaits (the caller is cancelled), the code after the await never
runs. If the client then never answers that request id, the entry stays in the pending map until the tunnel
connection ends.

TLC counterexample (12 steps, shortest found):

1. Connection c1 authenticates for s1 and registers.
2. Request r2 starts: pending[r2] = c1.
3. r2's caller is cancelled before the send completes, so the request never reaches the client.
4. c1 stays connected. The client cannot answer a request it never received; nothing removes pending[r2].

### When it can happen

- A caller is cancelled while the send is blocked (the outgoing channel is full because the client reads
  slowly), so the request is never delivered.
- A caller is cancelled after delivery and the client never answers that request. Our client always answers
  (its forward to the local service has a timeout, `crates/outpunch-client/src/lib.rs`), so this needs a
  client that is not ours, or a stuck one whose connection stays up.

Cancellation sources in this repo:
- Ruby binding: `handle_request` aborts the task when a Ruby exception interrupts the waiting thread, e.g.
  `Timeout::Error` or `Thread#kill` (`bindings/ruby/ext/outpunch/src/server.rs`, `task.abort()`).
- axum: whether hyper drops the handler future when the HTTP client disconnects is not verified here.

### Impact

Memory: one entry (request id and a oneshot sender) per affected request, held for the connection's
lifetime. No wrong responses, no hangs: every other safety property and `EveryRequestEnds` hold.

### Fix

`dispatch()` holds a `PendingGuard` whose `Drop` removes the request's pending entry, so removal runs on
every exit path: a response, an error, or the future being dropped. The error-path removal it replaces is
gone.

`Drop` cannot await, so the server state moved from `tokio::sync::Mutex` to `std::sync::Mutex`. The lock
was never held across an `.await`, so the critical sections are unchanged, and the compiler now rejects any
future code that would hold it across one in a spawned task. Tokio's documentation:

> "Contrary to popular belief, it is ok and often preferred to use the ordinary Mutex from the standard library in asynchronous code." — <https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html>

Correction to the earlier claim that "the compiler now rejects" holding the lock across an await: that is
true only for futures that must be `Send` (e.g. passed to `tokio::spawn`). The general guard is clippy's
`await_holding_lock` lint, which is in the default `suspicious` group, so `just clippy` (`-D warnings`)
rejects it.

Model: `Cancel` marks the future dropped and a separate `GuardDrop` step removes the entry, since the
destructor must take the lock and other steps can run first. What the model checks is that the guard, as
modeled, composes with everything else (`NoPendingAfterCancel`, `NoLeak`); it cannot show the Rust guard
is written correctly. The evidence for that is the Rust tests below, and the mutation in
`spec/mutations.txt` that removes the guard (both properties then fail).

### Reproduction

Two regression tests in `crates/outpunch/src/server_tests.rs`, one per cancellation window. Each starts
a request, aborts its task, and asserts the pending map is empty:

```sh
cargo test -p outpunch cancelled_request
```

- `cancelled_request_while_send_blocked_removes_pending`: the client's outgoing channel is full, so the
  request is never delivered (the TLC counterexample).
- `cancelled_request_awaiting_response_removes_pending`: the request is delivered and the client never
  answers.

Before the fix, both failed with `assertion failed: server.state.lock().await.pending.is_empty()`.

## F2: Ruby `next_outgoing` does not see an exception while it waits for the receiver lock

Status: **fixed.** Found by TLC (`spec/RubyGvl.tla`, `spec/RubyGvlRecv.cfg`, `ExceptionStopsCall`);
confirmed by reading `bindings/ruby/ext/outpunch/src/server.rs` (`next_outgoing`); reproduced by a
`pending` example in `bindings/ruby/spec/interrupt_spec.rb` ("stops the one waiting for the receiver when
it is interrupted with an exception"): the raised `RuntimeError` never reaches the second waiter, which is
still blocked a second later. With the fix the example passes (no longer pending).

`next_outgoing` awaits `outgoing.lock()` before its `tokio::select!` on the message and the cancel
`Notify`. If two threads call it on the same connection, the second waits for the lock; an exception raised
into it (`Thread#raise`, `Thread#kill`, `Timeout`) stores a `Notify` permit that nothing waits on until the
lock is free, so the exception is delayed until the first thread returns, which on an idle connection may
be never.

Not reachable through our adapters: the Rack adapter calls `next_outgoing` from one writer thread per
connection (`adapters/ruby/rack/lib/outpunch/rack/connection.rb`). A hazard for direct users of the native
gem.

Fix: the lock is acquired inside the `select!`, next to `cancel.notified()`, so an interrupt also stops a
thread still waiting for it. Dropping a waiting `lock()` neither takes the lock nor blocks later lockers
(vendor test `tokio_mutex_lock_cancelled_while_waiting_must_not_block_later_lockers`). Model: `LockCancel`
in `RubyGvl.tla`; `RubyGvlRecv.cfg` passes.

## F3: a dropped `run()` future skips cleanup

Status: **fixed.** Found by the adversarial review; modeled as `RunDropped` (`spec/RunDrop.cfg`:
`NoPendingOnFinishedConn` violated; `spec/RunDropLive.cfg`: `DeadUnregistered` violated); reproduced by
`dropped_run_unregisters_and_fails_pending`, which now passes.

If the `run()` future is dropped while it awaits (task abort, `select!`, runtime shutdown), nothing after the
await runs, including `cleanup_connection`. The service stays routed to the dead connection: in-flight
requests wait for the full timeout (504, not 502; `NoTimeoutAfterDisconnect` also fails, see
`RunDropOther.cfg`) and every later request to that service times out too, until another client registers.

In-repo exposure is low: the Ruby binding closes the connection and waits instead of dropping `run()`, the
Node binding cannot cancel it, and axum drops it only at shutdown. A public-API hazard.

Fix: once registered, `run()` holds a `CleanupGuard` whose `Drop` calls `cleanup_connection`, as F1's
guard does for requests. Covers task abort and runtime shutdown (vendor tests
`tokio_abort_must_run_destructors_of_the_tasks_locals`,
`tokio_runtime_shutdown_must_run_destructors_of_unfinished_tasks`). Model: a dropped `run()` in the relay
loop goes to `Cleanup`; `RunDrop.cfg` and `RunDropLive.cfg` pass.

## F4: a dead drain task leaves the service registered

Status: **fixed.** Found by the adversarial review; modeled as `DrainDeath` (`spec/DrainDeath.cfg`:
`DeadUnregistered` violated); reproduced by `dead_callback_unregisters_service`, which now passes.

If the `on_message` callback panics, the drain task dies. Requests to the service then fail: the first one
waits for the full timeout, later ones fail with `ConnectionLost` (502). The service keeps reporting
connected, and since the client's socket is still up it never reconnects. The in-repo callbacks do not
panic; this hits embedders.

Fix: the drain task holds a `CloseOnDrop` guard, so however it ends, including a panic, the connection
closes and `run()` cleans up. A panicking task runs its destructors and leaves the runtime running (vendor
test `tokio_panicking_task_must_run_destructors_and_leave_the_runtime_running`). With `panic = "abort"`
the process ends instead, which needs no fix. Model: `DrainClose` after `DrainDeath`; `DrainDeath.cfg`
passes.

## F5: an adapter delivers a request after outpunch has already answered its caller

Status: **fixed** in the axum adapter, the Ruby binding and the Node adapter. The memory side is now F7.
Found by TLC (`spec/AxumBridge.tla`: `AxumLate.cfg` violated `NoLateWrite`); reproduced by `timed_out_requests_are_not_delivered_later` in `crates/outpunch-axum/src/lib_tests.rs`.
Before the fix it failed with all 200 requests reaching the client after every caller got 504 (4 of 4
runs); it allows up to a quarter, since requests already in the socket buffers may still arrive. With the
fix it passes (4 of 4 runs), and fails again with 200 of 200 when the check is disabled. Ruby:
`bindings/ruby/spec/server_spec.rb`, "does not hand out a request whose caller already timed out", passes
with the fix and fails without it. Node: `bindings/node/tests/server.test.ts`, "timed-out requests must not
be delivered to a client that reads again later", the same scenario as axum: fails with 200 of 200 on the
old adapter, passes with the fix (3 of 3 runs).

### The problem

Outpunch should never deliver a request after it has answered that request's caller. Today it can:

1. A caller sends an HTTP request to the server; the server forwards it toward the tunnel client through
   the axum adapter's outgoing queue (`handle_ws` in `crates/outpunch-axum/src/lib.rs`).
2. The tunnel client is not reading, so the request waits in that queue. That waiting is correct.
3. After the request timeout (25 s by default) the server answers the caller itself with 504 and drops
   the request from its pending map: as far as outpunch is concerned, the request is finished.
4. The request is still in the adapter's queue, and nothing removes it.
5. When the tunnel client reads again, the adapter writes it, and the local service runs it, possibly
   long after the 504.

This is a property of one request, decidable from state outpunch already has (the pending map); it does
not depend on how requests relate to each other.

### Impact

- A 504 normally means "unknown, it may or may not have run" (as from any gateway), and callers should
  retry only idempotent requests. What is specific here is the delay: the request can run arbitrarily
  long after the 504, when the client recovers, instead of within about the timeout. All requests queued
  during the stall run together at that point.
- Memory: see F7.

### Fix

Change only the write side of the adapter's queue; keep the push model, the callback API and the
unbounded queue:

1. Core: a read-only check, `OutpunchServer::is_pending(&msg) -> bool`: is the request in this message
   still in the pending map? The pending map holds a request from the moment it starts until outpunch
   settles it (response relayed, timeout, cancellation, or disconnect), so "pending" means exactly "the
   caller has not been answered yet". It says nothing about the client. Every other message type
   (`auth_ok`, `auth_error`) counts as pending, so it is always sent.
2. axum's write task: after taking the next message off the queue (`msg_rx.recv()`, which already removes
   it), write it only if it is pending; otherwise drop it:

   ```rust
   while let Some(msg) = msg_rx.recv().await {
       if !server.is_pending(&msg) { continue; }   // caller already answered: drop it
       if ws_sink.send(WsMessage::text(msg)).await.is_err() { break; }
   }
   ```
3. Ruby binding: `Connection#next_outgoing` (`bindings/ruby/ext/outpunch/src/server.rs`) takes messages
   off its queue the same way and skips those that are not pending, so the Rack and Rails adapters never
   see them. Their write (`websocket-driver` to the socket) blocks like axum's, so the same residuals
   apply.
4. Node: the adapter (`bindings/node/js/server.ts`) used to call `ws.send` from the `onMessage` callback
   as soon as a message arrived. `ws.send` never waits, so with a client that is not reading every frame
   went into the `ws` library's and Node's socket write buffers, where nothing can remove it. The adapter
   now keeps its own queue and has one message on the socket at a time: the next one is taken when
   `ws.send`'s callback reports the previous frame written, and dropped if `server.isPending(msg)` (new in
   the binding) is false. Same residuals as axum.

The check reads only `type` and `request_id` (`protocol::request_id_of`); it does not copy the body
(except Node's string crossing into Rust).

Removal is lazy: a stale request stays in the queue until it reaches the front, then is dropped instead
of written. Removing it at timeout time would mean deleting from the middle of the FIFO, which the tokio
channel does not support.

Residual cases, by design:
- One message per connection may still arrive late: the one already being written when its request
  times out (a write to a stalled socket can block). Today it is every queued request.
- A request already in the kernel socket buffers before its 504 can still run later; only the protocol
  could close that (e.g. a request deadline the client enforces).
- The check scans each message to read its `request_id` (bodies can be up to 10 MB): linear in size,
  cheap next to writing the same bytes, not free.
- Memory is not addressed: F7.

Model: `AxumBridge.tla` splits the write task into `WriteTake` (`recv` plus the check) and `WriteSend`
(`ws_sink.send`, which waits while the peer is stalled). `NoLateWrite` allows exactly the first residual
(a request whose caller was answered while the write task held it) and passes (`AxumLate.cfg`); the
strict form, `NoLateWriteAtAll`, is violated (`AxumInFlight.cfg`), which shows that residual is real.
`spec/mutations.txt` removes the check and `NoLateWrite` catches it.

Neither binding is modeled; the Ruby and Node changes are covered by their tests, not by TLC.

Alternative considered: have the adapter pull from a bounded core channel (backpressure) with the same
check. It also bounds memory, but changes the adapter API; not needed to fix the problem above.

## F6: a peer that never reads keeps the write task and socket alive

Status: **fixed.** Found by TLC (`spec/AxumStalled.cfg`: `WriteTaskEnds` violated with a peer that stays
stalled; holds when a stalled peer eventually reads or errors, `AxumBridge.cfg`); confirmed by reading
`handle_ws`. Reproduced by `socket_of_a_finished_connection_closes_even_if_the_client_never_reads`: fails
without the fix, passes with it.

After `run()` returns, `handle_ws` waits up to 100 ms for the write task, then drops its `JoinHandle`
without aborting it. A write task blocked in `ws_sink.send` on a peer that is alive but never reads (the
TCP window stays at zero) keeps the task and the socket open indefinitely.

Fix: `handle_ws` aborts the write task when the 100 ms flush wait expires. Model: `WsReturns` ends the
writer. `AxumStalled.cfg` checks it with F7's write timeout switched off (`WriteTimeouts = FALSE`), since
that timeout would otherwise also end the task and hide whether this fix works.

## F7: a client that stays connected but stops reading makes server memory grow without limit

Status: **fixed**, with memory bounded by the request timeout rather than by the requests in flight.
Found by TLC (`spec/AxumStale.cfg`: `QueueOnlyLive` violated); split out of F5.

With a client that stays connected but never reads, the service stays registered, so new requests keep
being routed to it. Each one waits in the adapter's unbounded outgoing queue (axum: `msg_tx`; Ruby: the
binding's channel; Node: the adapter's queue), body included, after its caller got 504. The F5 check
does not help: the write task is stuck on one blocked write and never reaches the stale messages behind it.
Memory grows by about one request body per request until the server runs out, which takes down every
service on it.

The unbounded queue itself is intended: it fixed "The axum adapter silently dropped requests when a client
read slowly" (CHANGELOG v0.4.0).

Fix: a socket write that takes longer than the request timeout (`OutpunchServer::request_timeout`, 25 s by
default) closes the connection, so the service is unregistered, its queue is freed, and new requests fail
fast with 502 until the client reconnects. A client that cannot take one message within a caller's whole
time budget is not serving anyone.
- axum: `tokio::time::timeout` around `ws_sink.send`.
- Rack/Rails: `Connection#write` uses `write_nonblock` and `wait_writable` with a deadline (vendor specs
  in `adapters/ruby/rack/spec/vendor/ruby_io_spec.rb`; `IO#timeout=` needs Ruby 3.2, the gems support 3.1).
- Node: a timer per frame, cleared by `ws.send`'s callback; on expiry the connection is closed and the
  socket terminated.
- Native Ruby gem (`Outpunch::Connection`, for code that writes the socket itself): it does not own the
  socket, so it watches the queue instead. When a message is queued while the one at the front has waited
  longer than the request timeout for `next_outgoing`, the connection is closed. Checked at enqueue, which
  is when a stalled consumer would make memory grow; an idle connection with nothing queued is never
  closed. Uses the new `Connection::downgrade` in the core so the callback does not keep its own
  connection alive. Spec: `bindings/ruby/spec/server_spec.rb`, "closes the connection when a message
  waits for next_outgoing longer than the request timeout" (fails without the check), plus a control
  that a consumer taking each message in time is kept.

Reproductions: axum `client_that_stops_reading_is_disconnected`, Rack `connection_spec.rb` ("closes the
connection once a write has waited longer than the request timeout"), Node `server.test.ts` ("client
that stops reading must be disconnected after a write waits past the timeout"). Each fails without the
timeout. Model: `WriteTimeout` and `StalledPeerUnregistered` (`AxumStuck.cfg`, passes).

Remaining limit (sizing, not correctness): under overload, memory per connection is about **arrival
rate × request timeout**, bodies included. Two cases reach it:
- A client that stops reading: requests queue until the write timeout fires (up to one timeout).
- A client that keeps reading, but slower than requests arrive (a weak downlink with large uploads, or a
  CPU-starved client): no single write takes 25 s, so the timeout never fires. Requests at the back wait
  past their timeout and their callers get 504; F5 drops them almost instantly when they reach the front,
  so the queue holds about what arrived within the last timeout.

Callers get 504 in both cases, as from any gateway in front of a link slower than its traffic. The
memory matters only when it approaches the server's limit: with 10 MB bodies, 10 MB/s of sustained
uploads × 25 s is about 250 MB, which is half of a 512 MB Heroku standard-1X dyno; one that runs out of
memory restarts and takes every service with it. With small API bodies it is a few MB. `AxumStale.cfg`
(`QueueOnlyLive`) fails for this reason. A bounded queue ("Bounded Outgoing Queue" in `docs/future.md`)
would cap it lower and turn some 504s into fast 503s.

Direct users of the native Ruby gem are covered by its queue-side check (above), not a socket write
timeout.

## F8: a half-open connection goes unnoticed for about 15 minutes

Status: **fixed.** Found in production, not by a model: two StormSnap tunnel outages on 2026-10-24
(healthchecks.io alerts at 9:20 and 10:00 local, each about 19.5 minutes).

Evidence (the client's pod logs; mordor has no log drain, so the server side is gone):
- The client logged `connection closed` at 16:37:52 and 17:16:58 UTC and was authenticated again 5 s
  later each time, matching the two recoveries. The pod did not restart (0 restarts since 2026-10-23).
- No mordor release in the window, so not a deploy. Most of the client's other closes that week
  reconnected within 5 s: the server closed them cleanly.
- The client pinged every 30 s but never checked for a pong and had no read timeout. When the server
  side vanished without closing the socket, the pings sat unacknowledged until TCP gave up:
  `tcp_retries2` is 15 in the pod, about 15.4 minutes, which with the 5-minute uptime check matches the
  outage length.

The server had the mirror-image gap: a client that disappears without closing stays registered, and
requests to it wait for their timeout, until a write to it fails.

Fix:
- Client (R31): anything received (a pong or any message) resets an idle timer; after `idle_timeout`
  (75 s, with a ping every `ping_interval`, 30 s) the client drops the connection and reconnects. Writes
  have the same limit, so a blocked write cannot stop the check. Both are `ClientConfig` settings, also in
  the Python and Ruby bindings and the CLI (`--ping-interval`, `--idle-timeout`).
- Server (R30): the core keeps the time the client was last heard from (`push_message`, and
  `Connection::heartbeat`, which the axum, Node and Rack adapters call for every WebSocket ping) and
  evicts the client after `client_idle_timeout` (90 s). Existing clients already ping every 30 s, so they
  are not evicted.
- Model: `Tunnel.tla` lets a client go silent forever (`Silences`) and the relay loop evict it
  (`IdleEvict`); `Idle.cfg` checks `SilentUnregistered` together with the safety properties (an evicted
  connection's cleanup must not unregister a newer one, its requests fail with 502). A mutation that
  removes eviction is caught. The model has no clocks, so "a client that pings is never evicted" is a
  timing assumption, tested in Rust and in each adapter, not checked by TLC.
- Trace validation: `silent_client_is_evicted_after_the_idle_timeout` is in the trace set.

Heroku: a ping sent through the router to production mordor comes back as a pong with the same payload
(`crates/outpunch-client/tests/vendor_heroku.rs`). Whether the dyno or the router produced that pong is
not shown, so "the ping reaches the dyno" is still unverified (`vendor-claims.toml`). If it did not, the
server would evict idle clients every 90 s; check after deploying by leaving the tunnel idle for a few
minutes.
