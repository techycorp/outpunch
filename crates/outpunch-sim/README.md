# outpunch-sim

Runs a local outpunch setup and replays traffic through it, so you can watch what each part logs.
Not published; a development tool.

```sh
just sim                                   # the sample traffic, with short timeouts
just sim path/to/traffic.ndjson            # your own file, same short timeouts
just sim path/to/traffic.ndjson --help     # all options (pass any to replace the defaults)
```

## What runs

| Part | What it is | Log |
|---|---|---|
| server | the axum example (`crates/outpunch-axum/examples/server.rs`), a child process | `server.log` |
| client | the `outpunch-client` CLI, a child process | `client.log` |
| service | a stand-in local service the client forwards to, in the simulator | `service.log` |
| sim | the simulator itself: steps, events, each request's result | `sim.log` |

Every line from every part is also written, interleaved, to `all.log` and printed to the terminal as
it arrives:

```
+   2535ms sim     | event: killing and restarting the client
+   2548ms client  | connecting to ws://127.0.0.1:63196/ws...
```

The `+…ms` prefix is when the simulator captured the line (since it started); the rest is exactly
what the part wrote. Lines a child writes to stdout are marked `[stdout]`; outpunch writes to
stderr. Output goes to `target/sim/` (overwritten each run; `--out` to change), along with
`results.ndjson` (one line per request: status, latency, error) and `summary.txt`.

## Traffic file

One JSON object per line; blank lines and `#` comments are skipped. `at_ms` is when to run the step,
counted from the moment the tunnel is up. Steps at the same time run concurrently.

Requests (`method` defaults to GET; `path` is relative to the tunnel, so `/hello` is
`/tunnel/sim/hello`):

```json
{"at_ms": 0, "path": "/hello"}
{"at_ms": 50, "method": "POST", "path": "/upload", "body": "hi", "headers": {"x-trace": "1"}}
{"at_ms": 100, "path": "/slow", "reply": {"status": 503, "delay_ms": 2000, "body": "busy"}}
```

`reply` tells the stand-in service how to answer that request (default: 200 "ok" at once). It
travels as `x-sim-*` headers through the tunnel.

Events:

| Event | Effect |
|---|---|
| `{"at_ms": 1000, "event": "client_pause", "ms": 8000}` | SIGSTOP the client for `ms`, then SIGCONT: it neither pings nor reads, like a frozen or half-open peer |
| `{"at_ms": 2000, "event": "client_restart"}` | kill the client (no close frame) and start a new one |
| `{"at_ms": 3000, "event": "server_restart"}` | kill the server and start a new one on the same port |

## Log level

The server and client log at `info`. `RUST_LOG` reaches both, so
`RUST_LOG=info,outpunch=debug,outpunch_axum=debug,outpunch_client=debug just sim` adds outpunch's
debug events (plain `RUST_LOG=debug` also adds the libraries' own, which are noisy).

## Timeouts

The real defaults are slow to watch (request 25 s, client evicted after 90 s silent, client pings
every 30 s and reconnects after 75 s). `just sim` passes short ones instead:
`--request-timeout 3 --client-idle-timeout 4 --ping-interval 1 --idle-timeout 3`. Passing any
options replaces all four, so pass the full set you want.
