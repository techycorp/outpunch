# TLA+ Models of Outpunch

Formal models of outpunch's concurrency, checked with TLC, against [REQUIREMENTS.md](REQUIREMENTS.md).
Results are in [FINDINGS.md](FINDINGS.md). What the runs actually do, with the state counts, is in
[HOW-IT-WORKS.md](HOW-IT-WORKS.md).

| Model | What it models | Status |
|---|---|---|
| `Tunnel.tla` | The server core, `crates/outpunch/src/server.rs` | Built code; trace-validated against 16 core unit tests |
| `AxumBridge.tla` | One connection through the axum adapter and the core's outgoing path | Built code |
| `RubyGvl.tla` | The Ruby binding's GVL and interrupt call protocol (`bindings/ruby/ext`) | Built code |
| `MultiClient.tla` | Several clients per service with instance IDs and eviction ([notes/multi-client.md](notes/multi-client.md)) | Design only, not built |

## Running

```sh
just tla-all       # every model in spec/models.txt, checked against its expected outcome (~50 min, all cores)
just tla-mutants   # known bugs in spec/mutations.txt, each must be caught (~20 s)
just tla-traces    # trace validation: 16 core unit-test executions must be behaviors of Tunnel.tla (~15 s)
just tla           # spec/Tunnel.cfg only; `just tla <cfg> [TLC args]` for another Tunnel config
```

Tooling, all generic (any spec, any repo):

- `scripts/tla/tlc <spec> <cfg> [args]`: downloads the pinned `tla2tools.jar` to `~/.cache/tlaplus/`,
  verifies its SHA-256, runs TLC with Java from mise.
- `scripts/tla/check-all <manifest>`: runs each model with its expectation: `pass`, `fail:<property>` (a
  documented finding; the named property must be the one violated), or `reach:<invariants>` (non-vacuity
  witnesses, each checked in its own run so one cannot mask another). Extra args go to TLC, e.g.
  `-simulate` for random simulation.
- `scripts/tla/mutants <file>`: applies each listed text edit (a known bug) to a spec and checks that the
  named property catches it.

Trace validation (`spec/TunnelTrace.tla`, `spec/trace/check_traces.py`) is specific to `Tunnel.tla`: the
core, built with the `trace` feature (`crates/outpunch/src/trace.rs`), writes one event per model step;
the script runs each traced test, converts the trace, and asks TLC whether some behavior of the model
takes exactly those steps. Events also carry their effect (which request a relay completed, what cleanup
unregistered and failed, which connection a register replaced), checked against the model's state. It
reports the first event the model cannot take. Wrong models are rejected: a model that forbids
`push_message` before `run()`, and one whose `Kick` does not close the old connection.

## Tunnel.tla configs

| Config | Expect | Checks |
|---|---|---|
| `Tunnel.cfg` | pass | All safety invariants; `EveryRequestEnds`, `NoLeak`. 2 conns, 1 service, 2 requests, 2 messages each |
| `EmptySecret.cfg` | pass | Safety with an empty server secret (R2) |
| `Large3Conns.cfg` | pass | Safety, 3 conns, 2 services, 1 request (three-way replacement, cross-service), symmetry |
| `Large2Services.cfg` | pass | Safety, 2 services, 2 requests (isolation across services), symmetry |
| `LargeMsgs.cfg` | pass | Safety, 3 messages per connection (one connection answers both, duplicates, junk then response), symmetry |
| `LargeSim.cfg` | pass | Random simulation (200k behaviors, depth 80) of 3 conns, 2 services, 3 requests, 3 messages: not exhaustive |
| `Witness.cfg` | each violated | Reachable: success, timeout, disconnect 502, NotConnected, replacement, cancellation, a dropped future with its entry still pending |
| `Flapping.cfg` | `Settles` violated | Two eager clients for one service replace each other forever (known consequence, R9) |
| `RunDrop.cfg`, `RunDropLive.cfg` | pass | F3 fix: a dropped `run()` still cleans up its pending entries and unregisters |
| `RunDropOther.cfg` | pass | With `run()` drops and drain deaths, the other properties still hold |
| `DrainDeath.cfg` | pass | F4 fix: a dead drain task closes its connection, which then unregisters |
| `Idle.cfg` | pass; `NeverIdleEvicted` reachable | F8 fix: a client gone silent on a half-open socket is evicted (`SilentUnregistered`), with the safety properties |

Exhaustive 3 connections with 2 requests did not finish (over 17M states and growing), hence the split.

## Other configs

| Config | Expect | Checks |
|---|---|---|
| `AxumBridge.cfg` | pass | Requests end; the write task ends if a stalled peer eventually reads or errors |
| `AxumWitness.cfg` | each violated | Reachable: delivery, drain end, write end, a write timeout |
| `AxumLate.cfg` | pass | `NoLateWrite`: no request written after its caller was answered, except the one already being written (F5 fix) |
| `AxumInFlight.cfg` | `NoLateWriteAtAll` violated | F5 residual: the request already being written |
| `AxumStale.cfg` | `QueueOnlyLive` violated | F7 residual: stale requests stay queued until the write timeout fires |
| `AxumStalled.cfg` | pass | F6 fix: with a peer that never reads (and no write timeout), the write task still ends with the connection |
| `AxumStuck.cfg` | pass | F7 fix: `StalledPeerUnregistered`, a client that stops reading is disconnected |
| `RubyGvl.cfg` | pass | Ruby binding: push_message + handle_request; `ResultOnlyIfRan`, `NoDuplicateOp`, `RaisedOnlyByException`, `NoEndWithoutResult`, `EveryCallEnds`, `ExceptionStopsCall`, `NoLeak` |
| `RubyGvlRecvSafety.cfg` | pass | Two threads in `next_outgoing` on one connection: safety |
| `RubyGvlRecvOne.cfg` | pass | One thread in `next_outgoing`: safety, every call ends, exceptions stop it |
| `RubyGvlRecv.cfg` | pass | F2 fix: an exception stops a thread still waiting for the receiver lock |
| `RubyGvlPre052Skip.cfg`, `RubyGvlPre052Wakeup.cfg`, `RubyGvlNoGuard.cfg` | violated | Mutations: the two pre-v0.5.2 bugs (CHANGELOG v0.5.2) and the core without F1's fix are caught |
| `RubyGvlWitness.cfg` | each violated | Reachable: ok, raised, skipped, cancelled, retried then ok, aborted |
| `MultiClient.cfg`, `MultiClientLiveness.cfg`, `MultiClientSafety.cfg`, `MultiClientEmptySecret.cfg` | pass | Design model; see [notes/multi-client.md](notes/multi-client.md) |
| `MultiClientWitness.cfg` | each violated | Coexistence, same-ID replacement (also of a live client), eviction, success, routing to a dead socket |

## How Tunnel.tla maps to the code

- Every section of `server.rs` that holds the server mutex is one atomic action; every `.await` outside it
  is an interleaving point. Trace validation checks this mapping on real executions.
- Tunnel clients, the adapter and HTTP callers are the unconstrained environment: create a connection,
  push messages (before or after `run()` starts), close, stall, cancel a request, drop `run()` (`RunDrops`),
  kill the drain task (`DrainDies`). Clients may name any request sent to any connection, so colluding
  clients are covered.
- Server-side steps are weakly fair; environment steps are not.
- tokio's `timeout` polls the inner future first, so a resolved response always beats the timer
  (<https://docs.rs/tokio/latest/src/tokio/time/timeout.rs.html>: "First, try polling the future").
- Abstracted: request ids are distinct values (UUIDs); the outgoing channel and drain task are one `Send`
  step that may never happen (modeled in detail in `AxumBridge.tla`); timers fire nondeterministically;
  channel capacities are not modeled.

## Requirement coverage

"Property" means a TLC-checked invariant or temporal property, with a mutation in `spec/mutations.txt`
that it catches where noted. "Modeled" means the behavior is in the model and exercised by trace
validation, but no property states it.

| Requirement | How |
|---|---|
| R1 auth before routing | `RoutedOnlyIfAuthed` (mutation: accept any token) |
| R2 empty secret | `EmptySecretRejectsAll` (mutation: ignore the empty secret) |
| R3, R5 first message must be a valid auth | Modeled in `Auth`; trace-validated |
| R4 auth timeout | Modeled in `AuthTimeout`; trace-validated |
| R6 one service per connection | Modeled (auth names one service) |
| R7, R8 routing, NotConnected | Modeled in `Start`; NotConnected reachable (witness) |
| R9 newest wins, old closed | `ReplacedIsClosed` (mutation: Kick does not close); flapping in `Flapping.cfg` |
| R10 stale disconnect | `NoStaleRemoval` (mutation: no conn_id check) |
| R11 unique ids | Abstraction (distinct model values) |
| R12, R13 right caller, right connection | `ResponseFromTarget` (mutation: any connection answers), `PendingOwnedByTarget` |
| R14 exactly one outcome, never hangs | `EveryRequestEnds` (mutation: timer never fires) |
| R15, R16 timeout covers send and wait | `NoTimeoutOverResponse` (mutation: timer beats an arrived response); `EveryRequestEnds` relies on the timer while `Send` has no fairness; trace-validated (`request_times_out_over_a_real_connection`) |
| R17 disconnect fails pending with 502 | `NoPendingOnFinishedConn`, `NoTimeoutAfterDisconnect` (mutations: cleanup keeps entries; timer beats a response) |
| R18, R22 unknown or malformed messages ignored | Modeled in `Relay`; trace-validated (`connection_ignores_malformed_messages`) |
| R19 no leaks | `NoLeak`, `NoPendingAfterCancel` (mutation: no PendingGuard); trace-validated (`connection_keeps_serving_after_a_request_is_cancelled`) |
| R20 stalled client doesn't block others | By construction (`Send` outside the lock); `Large2Services.cfg` covers two services |
| R21 reconnect works immediately | `NoSpuriousNotConnected`: no NotConnected while the newest registrant runs (mutation in `Start`). It does not involve a reconnect specifically |
| R23 replaced connection's requests | Modeled (no resend); `Cleanup` fails them |
| R24 run() cleans up on every exit | `NoPendingOnFinishedConn`, `DeadUnregistered` with `RunDrops` (mutation: no cleanup guard). `DeadUnregistered` holds trivially without `RunDrops`/`DrainDies`, so it is checked where they are on |
| R25 dead drain task | `DeadUnregistered` with `DrainDies` (mutation: the drain task does not close its connection) |
| R26 adapter queue holds only live requests | `QueueOnlyLive` violated (F7 residual) |
| R27 no late writes | `NoLateWrite` (mutation: no `is_pending` check) |
| R28 socket lifetime | `WriteTaskEnds` (mutation: the write task is not aborted) |
| R29 stalled client disconnected | `StalledPeerUnregistered` (mutation: no write timeout) |
| R30 silent client evicted | `SilentUnregistered` in `Idle.cfg` (mutation: no eviction); trace-validated (`silent_client_is_evicted_after_the_idle_timeout`). "A pinging client is never evicted" is a timing assumption, tested in Rust and the adapters |
| R31 client reconnects on a silent server | Client code, not modeled; tested in `crates/outpunch-client` |

## Keeping models in step with the code

A model is a separate artifact: changing the code does not change it. What links them:

| Link | Covers | Enforced by |
|---|---|---|
| Trace validation (`just tla-traces`) | `Tunnel.tla` against `crates/outpunch/src/server.rs`: a code change the model does not allow makes a trace fail, naming the first event the model cannot take | running it |
| Expected outcomes (`spec/models.txt`) | every config must give its expected result; a finding's config must fail on the named property | `just tla-all` |
| Mutations (`spec/mutations.txt`) | the properties still catch known bugs after a model change | `just tla-mutants` |
| Reproductions | each open finding has a Rust `#[ignore]` test or an RSpec `pending` example that fails until fixed | RSpec `pending` errors once it passes; a Rust `#[ignore]` must be removed by hand |

Not linked: `AxumBridge.tla` and `RubyGvl.tla` have no trace validation, so changes to the axum adapter
or the Ruby binding reach their models only by hand. Nothing yet fails a change to code that leaves its
model untouched, and none of these checks runs in CI (backlog: `docs/future.md`).

When a change touches modeled behavior, in the same commit:

1. Change the code and its tests; remove the `#[ignore]` (or `pending`) from a fixed finding's reproduction.
2. Change the model to match. Model files: `server.rs` → `Tunnel.tla`; `crates/outpunch-axum/src/lib.rs`
   → `AxumBridge.tla`; `bindings/ruby/ext/outpunch/src/` → `RubyGvl.tla`.
3. Update `spec/models.txt`: a fixed finding's config goes from `fail:<property>` to `pass`.
4. Add a mutation to `spec/mutations.txt` that undoes the fix; the property must catch it.
5. Update [FINDINGS.md](FINDINGS.md), [REQUIREMENTS.md](REQUIREMENTS.md) if behavior changed, and the
   CHANGELOG.
6. Run `just tla-traces`, `just tla-mutants`, `just tla-all` and the tests.

## Limits

- Bounded: exhaustive checks cover at most 3 connections and 2 requests; larger instances are sampled.
- Trace validation covers 16 core tests that use only the public API (listed in
  `spec/trace/check_traces.py`) on a single-threaded runtime, including a timeout and a cancellation. Not
  traced: send failures, drain death and dropped `run()` (F3, F4 have no passing executions yet), tests
  that edit internal state (fake service handles), and the adapters. A trace shows one execution is
  allowed by the model, not that all are.
- `MultiClient.tla` keeps its own copy of the request lifecycle; changes to `Tunnel.tla` are not carried
  over automatically.
- The model is kept in step with the code by hand; trace validation and the mutation list are the checks
  that it has not drifted.

## Vendor assumptions

The models encode facts about third-party behavior. Verified (vendor tests, listed in `vendor-claims.toml`):
the tokio facts in `crates/outpunch/tests/vendor_tokio.rs` (timeout poll order, cancelled `send` delivers
nothing, a dropped oneshot sender fails the receiver, abort runs destructors, a `Notify` permit survives
for a later waiter, runtime shutdown drops unfinished tasks, a panicking task runs its destructors and
leaves the runtime running, a cancelled `Mutex::lock` does not block later lockers); run with
`just test-vendor`.

The adapter fixes rely on two more, verified the same way: `ws.send`'s callback fires only once the frame
is written (`bindings/node/tests/vendor/ws.test.ts`), and Ruby's `write_nonblock` / `wait_writable`
report a full socket and time out (`adapters/ruby/rack/spec/vendor/ruby_io_spec.rb`).

The Ruby C API semantics in `RubyGvl.tla`'s header are verified too, by
`bindings/ruby/spec/vendor/ruby_gvl_spec.rb` against the running Ruby, through a small C probe that calls
the C API directly (`spec/vendor/support/gvl_probe.c`): a pending interrupt (from `Thread#wakeup`, or the
VM timer while another thread waits for the GVL) makes `rb_thread_call_without_gvl2` skip its function;
`Thread#raise` and `Thread#wakeup` call the unblocking function and let the function return; after a raise
`rb_thread_check_ints` raises the exception, after a wakeup it raises nothing.

Not covered by a vendor spec: `Thread#kill` (assumed to behave like `Thread#raise`).

