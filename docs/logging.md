# Logging

Conventions for what outpunch logs and how. Scope for now: the Rust crates (`outpunch`,
`outpunch-axum`, `outpunch-client`) and the Rust binaries. The Python, Node and Ruby packages
follow later. Status: **agreed conventions, not yet implemented**; `just sim` shows the current
output (see [crates/outpunch-sim/README.md](../crates/outpunch-sim/README.md)).

## Mechanism

- **Libraries emit, binaries decide.** The library crates log through the
  [`tracing`](https://docs.rs/tracing) facade and never install a subscriber, print, or choose a
  format: outpunch runs inside other people's processes. `eprintln!` and `println!` are not used in
  library code.
- **Binaries install the subscriber.** The client CLI and the axum example server call
  `outpunch::logging::init_stderr` (feature `stderr-logging` of the `outpunch` crate; the client
  re-exports it as `init_stderr_logging`): stderr, filtered by `RUST_LOG` (default `info`).
- **Bindings must not go silent.** Today the client prints to stderr, and hosts such as Python rely
  on seeing those lines. When the bindings are done, each routes events into its host's logging; until
  then, a binding installs the same stderr formatter when it loads, so its output does not disappear.
- The `trace` cargo feature is unrelated: it records model steps for trace validation
  (`spec/README.md`) and stays off in normal builds.

## Format

One line per event: time, level, target (the module), message, then fields as `key=value`:

```
18:02:58.123456Z  INFO outpunch::server: service registered service=sim conn=2
18:03:02.456789Z  WARN outpunch::server: client silent; evicting conn=2 service=sim idle_s=90
```

- **Time:** UTC time of day with microseconds, `HH:MM:SS.ffffffZ`. Fixed width, so the level and
  message line up; no date, since platform logs (Heroku, Kubernetes) and log files carry it, and
  sub-millisecond ordering matters more here than the date.
- **Level:** padded to five characters (` INFO`, ` WARN`, `ERROR`), right after the time.
- **Message:** a short lowercase phrase saying what happened, the same text every time. Anything that
  varies goes in a field, never in the message, so lines can be searched and counted.
- **Values:** identifiers, numbers and fixed words are bare (`reason=idle`); free text that can
  contain spaces (`error`, a peer's message) is quoted (`error="connection refused"`). In code: `%`
  or a plain value for the first, `?` on a `String` for the second.
- **Color:** only when stderr is a terminal; log files and platform logs get plain text.
- **Fields** (snake_case, the same name everywhere):

| Field | Meaning |
|---|---|
| `conn` | server-side connection id (a counter, unique per server process) |
| `service` | service name; `-` on a server connection that has not authenticated yet |
| `req` | request id, first 8 characters |
| `method`, `path` | the tunneled request; path without the query string |
| `status` | HTTP status |
| `duration_ms` | how long the request or operation took |
| `reason` | why something ended or failed, from a fixed set of words per event |
| `error` | an underlying error's text |
| `failed` | how many in-flight requests a cleanup failed with 502 |

## Levels

| Level | Use for | Examples |
|---|---|---|
| `error` | a bug or broken invariant; needs a code change | the outgoing-message callback panicked |
| `warn` | something cost a caller a 502 or 504, or points at a misbehaving peer or misconfiguration | auth rejected, client evicted, connection replaced, request timed out |
| `info` | lifecycle and one summary line per request | connected, authenticated, registered, connection closed, `request` |
| `debug` | detail for investigating | ignored client messages, a stale request dropped (F5) |
| `trace` | not used | |

## Never log

- the secret or any auth token;
- request or response bodies;
- header values (they carry `Authorization` and cookies);
- query strings (they can carry tokens), so log `path`, not the full URL;
- anything from a client message beyond its type and request id.

## Events

The catalog. An event not listed here is added here first.

### Server (`outpunch`, `outpunch-axum`)

| Level | Message | Fields |
|---|---|---|
| info | `client authenticated` | `conn`, `service` |
| warn | `auth rejected` | `conn`, `reason` (`invalid_token`; `secret_not_set`, the server's own misconfiguration; `not_auth`, the first message is not an auth message or not valid JSON) |
| info | `auth not completed` | `conn`, `reason` (`timeout`, `closed`) |
| info | `service registered` | `service`, `conn` |
| warn | `service registered; older connection replaced` | `service`, `conn`, `replaced` |
| warn | `client silent; evicting` | `conn`, `service`, `idle_s` |
| warn | `client stopped reading; closing connection` | `conn`, `service`, `wait_s` |
| error | `outgoing message callback panicked; closing connection` | `conn`, `service` |
| info | `connection closed` | `conn`, `service`, `reason` (`closed`, `idle`, `dropped`, `replaced`, `drain_died`), `failed`, `duration_s` |
| warn | `request timed out` | `req`, `service`, `duration_ms` |
| debug | `no client for service` | `req`, `service` |
| debug | `ignored client message` | `conn`, `reason` |
| debug | `dropped request whose caller was already answered` | `req` |
| info | `request body too large` | `service`, `limit` |

### Client (`outpunch-client`)

| Level | Message | Fields |
|---|---|---|
| info | `starting` (CLI only) | `server` (URL without query), `service`, `forward_to` |
| info | `connecting` | `server` (URL without query), `service` |
| warn | `connect failed` | `error` |
| info | `authenticated` | `service` |
| error | `auth rejected` | `reason` (the server's message) |
| warn | `no message from the server; reconnecting` | `idle_s` |
| warn | `write to the server timed out; reconnecting` | `wait_s` |
| info | `connection closed` | `reason` (`server_closed`, `auth_rejected`, `idle`, `write_timeout`), `duration_s` |
| warn | `connection closed` | `reason=error`, `duration_s`, `error` (anything else: a reset, a protocol error) |
| info | `reconnecting` | `delay_s` |
| info | `request` | `req`, `method`, `path`, `status`, `duration_ms` (one line when it completes) |
| warn | `local service failed` | `req`, `status` (502 or 504), `error` |

## Testing

Lifecycle events are behavior: the tests for eviction, replacement, auth and close assert that the
event was emitted with its fields, using a capturing subscriber in the test. Per-request and
debug events are not asserted.
