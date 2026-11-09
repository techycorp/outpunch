# How the Model Checking Works

What actually happens when `just tla-all` runs, what the "millions of states" are, and how each kind of
check (model checking, witnesses, mutants, trace validation, vendor tests) contributes. Numbers are from
the runs on 2026-11-06, on this repo's models. The findings themselves are in [FINDINGS.md](FINDINGS.md).

## The one-sentence version

TLC takes the model (a state machine describing what the server, the clients and the callers can do),
starts from the initial state, and mechanically tries **every possible next step from every state it
can reach**, until there are no new states left. It checks the properties in every one of them. A
"state" is one complete snapshot of the system; "1.6 million states" means 1.6 million distinct
snapshots the system can actually get into.

## What a state is

In `Tunnel.tla` a state is the value of eight variables at one instant:

| Variable | Meaning | Example value |
|---|---|---|
| `services` | which connection each service routes to | `s1 -> c2` |
| `pending` | which connection each waiting request belongs to | `r1 -> c2, r2 -> None` |
| `conn` | per connection: phase (created, auth, relay, cleanup, done, ...), open or closed, buffered messages, service | `c1: relay, closed, inbox <<resp r1>>` |
| `caller` | per request: where its `dispatch()` is (sending, waiting, done, cancelled, ...) and its result | `r1: waiting` |
| `oneshot` | per request: the response channel (empty, has a value, closed) | `r1: empty` |
| `delivered` | requests a client has seen (what a client can answer) | `{r1}` |
| `authed`, `lastReg` | bookkeeping for the properties ("ghost" variables, no code counterpart) | |

A step is one action from the model: a client connects, a client pushes a message, the server
authenticates, registers, relays one message, cleans up, a request starts, its send completes, its timer
fires, its caller is cancelled, and so on. Each action corresponds to one section of `server.rs` that
runs without an `.await` (see [README.md](README.md), "How Tunnel.tla maps to the code"). From a typical
state several actions are possible, because many things can happen "next" in a concurrent system: that
branching is where the numbers come from.

## What TLC does with it

1. **Start** with the initial state (everything idle, nothing connected).
2. **Expand** a state: compute every state one step away. Each one computed counts as *generated*.
3. **Deduplicate**: TLC hashes each new state to a 64-bit fingerprint. If it has seen the fingerprint
   before, it drops the state; otherwise it is a new *distinct* state and goes on the queue.
4. **Check** every new distinct state against the invariants (the safety properties). A violation stops
   the run and prints the path that led there.
5. **Repeat** until the queue is empty. It works breadth-first: all states 1 step from the start, then
   all states 2 steps away, and so on. So the first violation it finds is reached by the **shortest
   possible path**, which is why counterexamples are short and readable.

The output lines mean:

| Line | Meaning |
|---|---|
| `7441309 states generated` | successor computations, duplicates included |
| `1630132 distinct states found` | unique states: the size of the reachable state space |
| `0 states left on queue` | exploration finished; the result covers every reachable state |
| `depth of the complete state graph search is 31` | the farthest state is 31 steps from the start (shortest path) |
| `Checking temporal properties for the complete state space` | the liveness phase, below |

Generated / distinct (about 4.6 for `Tunnel.cfg`) is how many different ways each state is reached on
average: two clients doing the same things in a different order often end in the same state.

## The runs, with numbers

| Config | Instance | Distinct states | Generated | Depth | Time | Result |
|---|---|---|---|---|---|---|
| `EmptySecret.cfg` | 2 conns, 1 service, 2 requests, secret empty | 15,876 | 70,309 | 15 | 1 s | pass |
| `Tunnel.cfg` | 2 conns, 1 service, 2 requests | 1,630,132 | 7,441,309 | 31 | 187 s | pass (safety and liveness) |
| `LargeMsgs.cfg` | as above, 3 messages per connection | 6,687,823 | 33,683,434 | 35 | 59 s | pass (safety, symmetry) |
| `Large3Conns.cfg` | 3 conns, 2 services, 1 request | 16,029,038 | 94,382,384 | 38 | 204 s | pass (safety, symmetry) |
| 3 conns, 1 service, 2 requests | (attempted) | over 17,500,000 and growing, 7.1M queued at depth 20 | | | stopped | did not finish |
| `LargeSim.cfg` | 3 conns, 2 services, 3 requests, 3 messages | not exhaustive: 200,000 random behaviors of up to 80 steps | | | ~70 s | no violation found |
| `Flapping.cfg` | 2 eager clients, reconnecting | 1,834 | 4,611 | | 1 s | `Settles` violated (expected) |
| `RunDrop.cfg` | `run()` may be dropped | 4,771,268 | 21,770,605 | | minutes | pass (F3 fixed; before the fix it stopped at 9,912 states, violated) |
| `RunDropOther.cfg` | `run()` drops and drain deaths, safety and liveness | 21,495,796 | 120,107,301 | 33 | 2 h 53 min | pass |
| `AxumBridge.cfg` | 1 connection, 2 requests, axum path | 2,166 | 8,570 | 19 | 1 s | pass |
| `AxumStale.cfg` | same | 199 when it stopped | 456 | 7 | 1 s | violated (F7) |
| `RubyGvl.cfg` | 2 Ruby threads (push, request) | 31,034 | 101,407 | 22 | 3 s | pass |

The full manifest (`just tla-all`, 55 checks across 4 models) took 48 minutes before the F3/F4 fixes. Since
then the configs that used to stop at their first violation explore everything: `RunDropOther.cfg` alone
takes about 3 hours, and the whole manifest about 5. The checks that fail keep TLC's output in
`target/tlc/logs/`.

### Why the numbers explode

Each extra actor multiplies the state space, because its state combines with everyone else's. Going
from 2 to 3 connections took the safety check from about 1.6M to 16M distinct states **even with
symmetry reduction**, and 3 connections with 2 requests never finished. This is the "state explosion"
problem, and it decides what can be checked exhaustively.

Two tools keep it in hand:

- **Symmetry reduction** (`SYMMETRY Symmetry` in the `Large*` configs). Connections c1, c2, c3 are
  interchangeable: a state where c1 is registered and c2 is closed is the same, up to renaming, as one
  where c2 is registered and c1 is closed. TLC stores one representative per group, a reduction of up to
  3! x 2! = 12 times for `Large3Conns.cfg`. It is sound for safety but not for liveness, so liveness runs
  do without it.
- **Random simulation** (`LargeSim.cfg`, `-simulate`). Instead of every state, TLC walks 200,000 random
  paths of up to 80 steps through a larger instance and checks every state on them. It cannot prove
  anything, but it reaches sizes the exhaustive search cannot.

The bet behind small instances is the *small scope hypothesis*: most concurrency bugs need only two or
three actors to show up. Every finding here appeared with two connections and one or two requests.

## Safety and liveness are checked differently

- **Safety** ("nothing bad ever happens", e.g. a response never reaches the wrong caller) is an invariant:
  a true/false test applied to every state as it is found. A failure is a finite path to a bad state.
- **Liveness** ("something good eventually happens", e.g. every request eventually ends) cannot be seen in
  one state. TLC first builds the whole graph of states and steps, then searches it for an infinite
  behavior that never does the good thing: a path into a **cycle** that loops forever. This is the
  "Checking temporal properties for the complete state space" phase, and the reason `Tunnel.cfg` (with
  three liveness properties) takes minutes while a larger safety-only run can take less.
- **Fairness** decides which cycles count. Without it, "the server simply stops doing anything" is a
  valid infinite behavior and every liveness property fails trivially. The model says server steps are
  *weakly fair* (a step that stays possible eventually happens) and the environment is not (a client may
  stall forever, a caller may never come back). So a liveness property passing means: it holds no matter
  what clients and callers do, as long as the server keeps running.

## What a failure looks like

When TLC finds a violation it prints the behavior, state by state. Two examples from this work:

**F1, a safety-style leak** (the first finding, 12 steps): c1 authenticates and registers; request r2
starts and is recorded as pending on c1; r2's caller is cancelled before the send completes; c1 stays
connected. The client never saw r2, so it can never answer it, and nothing ever removes the entry. TLC
reported it as a liveness violation of `NoLeak`: a cycle where the entry stays forever.

**Flapping, a liveness cycle** (`Flapping.cfg`): the loop TLC found, summarized:

| Step | `services[s1]` | c1 | c2 |
|---|---|---|---|
| 15 | c2 | reconnecting | relaying |
| 22 | c2 | authenticated, registering | relaying |
| 23 | **c1** | replacing c2 | relaying |
| 24-27 | c1 | relaying | closed, cleaned up, reconnecting |
| 30 | c1 | relaying | authenticated, registering |
| 31 | **c2** | relaying | replacing c1 |
| 32-34 | c2 | closed, cleaned up | relaying |
| back to 15 | | | |

Neither client ever closes on its own; newest-wins alone makes them displace each other forever.

## Runs that are supposed to fail

Several configs in `spec/models.txt` are expected to fail, and `check-all` treats a pass as an error:

- **Known findings** (`fail:<property>`): e.g. `RunDrop.cfg` had to violate `NoPendingOnFinishedConn`
  until F3 was fixed. When a fix lands, its line flips to `pass`. What remains: `AxumStale.cfg` and
  `AxumInFlight.cfg` (documented residuals of F7 and F5) and `Flapping.cfg` (a known consequence of R9).
- **Witnesses** (`reach:<invariants>`): an invariant like `NeverOk` ("no request ever succeeds") is
  written so that TLC *should* break it. Breaking it proves the model can reach a successful response at
  all. Without witnesses, a model that accidentally forbids everything interesting would pass every
  property and prove nothing. Each witness gets its own run, because TLC stops exploring past the first
  violation and one witness could otherwise hide another.

## Mutation checks: do the properties catch real bugs?

`just tla-mutants` plants 19 known bugs into a copy of the model, one at a time: 15 in `Tunnel.tla`,
3 in `AxumBridge.tla` and 1 in `RubyGvl.tla`. Each fix F1-F8 has one that undoes it; the others are
classic mistakes (accept any token, forget to close a replaced connection, drop the conn_id check in
cleanup, let the timer beat an arrived response, ...). It checks that the property meant to catch each
one fails. All 19 are caught. This is how the adversarial review's point "the auth check passes for any token" was
fixed and stays fixed: that mutant used to survive.

## Trace validation: is the model the code?

Everything above checks the model. Trace validation checks that the model matches the Rust:

1. The core, built with the `trace` feature, writes one line per model step as it runs, e.g.
   `{"ev": "start", "req": "...", "svc": "my-service", "conn": 1}` or
   `{"ev": "cleanup", "conn": 1, "unregistered": true, "failed": ["..."]}`.
2. `spec/trace/check_traces.py` runs 19 of the core's unit tests that way, then turns each trace (1 to 25
   events) into a TLA+ sequence.
3. TLC searches for a behavior of the model that takes exactly those steps, in that order, with the same
   effects. If it finds none, the script bisects to the first event the model cannot take.

All 19 traces are accepted, including a dropped `run()` (F3), a panicking callback (F4) and an idle
eviction (F8). To show the check has teeth, two wrong models were tried and rejected: one
that forbids pushing messages before `run()` starts (which the API allows, and three tests do), and one
whose replacement step does not close the old connection.

## Vendor tests: are the model's assumptions about tokio true?

The models take some tokio behavior as given: `timeout` prefers an inner future that is already ready, a
cancelled channel send delivers nothing, dropping a oneshot sender fails its receiver, aborting a task
runs destructors, a `Notify` permit survives for a later waiter. `crates/outpunch/tests/vendor_tokio.rs`
observes each against real tokio (`just test-vendor`), and each test was run once against a deliberately
wrong world to show it can fail. The Ruby C API assumptions in `RubyGvl.tla` are not yet verified this
way (see [README.md](README.md), "Vendor assumptions").

## What a pass means, and what it does not

A pass means: within the model's bounds, no ordering of steps, however unlikely, breaks the property.
That covers interleavings no test would ever hit by chance. It does not mean:

- that larger instances are fine (only sampled, by simulation);
- that the Rust matches the model beyond the 16 traced executions;
- that requirements nobody wrote down hold. The model only checks the properties in
  [REQUIREMENTS.md](REQUIREMENTS.md).

## Findings so far

| ID | Finding | Status |
|---|---|---|
| F1 | A cancelled request leaked its pending entry | fixed |
| F2 | Ruby `next_outgoing` ignores exceptions while waiting for the receiver lock | fixed |
| F3 | A dropped `run()` future skips cleanup; the service stays routed to a dead connection | fixed |
| F4 | A dead drain task (panicking callback) leaves the service registered | fixed |
| F5 | Adapters write requests to a slow client after their callers got 504 | fixed |
| F6 | A peer that never reads keeps axum's write task and socket alive | fixed |
| F7 | A connected client that stops reading makes server memory grow without limit | fixed (bounded by the request timeout) |
| F8 | A half-open connection goes unnoticed for about 15 minutes (production, 2026-10-24) | fixed |

Plus three design issues in the not-yet-built multi-client design, in
[notes/multi-client.md](notes/multi-client.md). Details, evidence and fix options for each are in
[FINDINGS.md](FINDINGS.md).
