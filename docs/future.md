# Outpunch — Future Work

Things explicitly deferred from phase 1. Not prioritized — just captured so nothing is lost.

## Standalone Server Binary

A standalone `outpunch-server` binary with its own HTTP + WebSocket server. The host app reverse-proxies `/tunnel/*` to it. Useful when embedding isn't an option. Also enables Docker image and Helm chart distribution.

## Multiple Clients Per Service

The server accepts multiple client connections for the same service name and round-robins between them. This is a natural consequence of deploying multiple instances with embedded clients.

**Tradeoff**: adding any form of request distribution means outpunch takes on load balancer responsibilities. Even simple round-robin raises questions: health checks? Connection weighting? Sticky sessions? Failure detection when one client disappears? Each of these grows scope significantly.

**Alternative**: keep outpunch as one client per service. If you need multiple instances, the standalone client sits in front of a real load balancer (Traefik, nginx, HAProxy) on the private side. Outpunch is the tunnel, the load balancer is the load balancer.

This alternative only works when the instances share a private network. It does not fit the target deployment: the server on Heroku with clients for the same service in different regions, reachable only over the public internet, so there is no private side to put a load balancer on. Multiple clients per service is therefore a real requirement for that deployment. Design notes: [spec/notes/multi-client.md](../spec/notes/multi-client.md).

**Current behavior**: one client per service, newest wins. When a second client authenticates for a service, it replaces the first, and the server closes the replaced connection and fails its in-flight requests with 502. This favors fast reconnects: a client whose old socket died without the server noticing can take over immediately. Two live clients misconfigured with the same service will repeatedly displace each other, which shows up as reconnect churn in their logs.

**Open questions**:
- If it accepts multiple, is simple round-robin enough, or does that create false expectations of load balancer behavior?

## Multiple Services Per Client

A single client registers for and forwards to multiple services. Already in the protocol spec (`"services": ["my-service", "pdf-gen"]`) and the Python client implementation, but not in phase 1 scope.

## Language Bindings

Client bindings use per-language FFI tools (not UniFFI — its Node.js support is experimental and Ruby support is deprecated). Python bindings via PyO3 are implemented. Remaining:

- **Node.js** (Napi-RS): mature, first-class async → Promise support, used by SWC/Rspack
- **Ruby** (Magnus + rb-sys): mature for sync, async requires manual reactor pattern (GVL release + tokio channel, proven by Temporal SDK)

Server-side adapters for other languages are written natively in each language:

- Ruby: `outpunch-ruby-rack` (Rack, Sinatra) and `outpunch-ruby-rails` — shipped, running on the Rust core via the native `outpunch-ruby` gem
- Python: `outpunch-asgi` (Django, FastAPI), `outpunch-wsgi` (Flask)
- Node.js: `outpunch-express`, `outpunch-hono`

## Additional Rust Adapters

`outpunch-actix`, `outpunch-warp`, etc. Each is a new adapter crate — no core changes.

## Binary WebSocket Frames

Alternative to base64-encoding binary bodies. The protocol currently uses JSON text frames with base64 for binary content. Binary WS frames would reduce overhead for large payloads (PDFs, images). Requires changes to both protocol types and the channel interface.

## TLS / mTLS

Mutual TLS for client authentication as an alternative to shared secrets. Stronger security model but more complex to configure and deploy.

## Service Authorization

Enforce which services a given client token is allowed to register for. Currently the shared secret grants access to all services. Service-level authorization would let you issue scoped tokens.

## Hot-Reload Service Configuration

The client detects changes to its service configuration file and subscribes/unsubscribes without restarting. Already implemented in the Python client.

## Standalone Client Distribution

Docker image, Helm chart, and pre-built binaries for the standalone client. Makes it easy to deploy the client as a sidecar or standalone container.

## Bounded Outgoing Queue

The axum, Node and Ruby adapters queue messages for a client without a limit. A client that reads slower than requests arrive, for a sustained period, makes server memory grow by roughly one request body per queued message. Since F7's fix, memory under overload is about arrival rate × request timeout, bodies included
([spec/FINDINGS.md](../spec/FINDINGS.md), F7): a sizing concern for small servers (a 512 MB dyno) with
sustained large uploads. A bounded queue that fails new requests fast (503) when full would cap it lower.

## Request Deadline in the Protocol

After a 504, a request can still reach the client and run: the one being written when its caller timed
out, and anything already in the socket buffers (F5 residual in
[spec/FINDINGS.md](../spec/FINDINGS.md)). No adapter can prevent that. Send a deadline with each request
and have the client drop requests that arrive after it. Needs a protocol change in every client.

## Client Concurrency Limit

The client forwards every tunneled request to the local service at once, with no cap. Add an optional `max_concurrent` setting so a burst can't overload a small local service.

## Warn on Connection Replacement

When a newer client replaces an existing one for the same service, log a warning on the server. Two live clients misconfigured with the same service name otherwise only show up as reconnect churn in the client logs.

## Precompiled Ruby Gems

The native `outpunch-ruby` gem is published as source and compiles its Rust extension on install, so every app using `outpunch-ruby-rack` or `outpunch-ruby-rails` needs a Rust toolchain at install time (a problem for many deploy platforms). Publish precompiled platform gems with rb-sys's cross-compilation tooling, keeping the source gem as a fallback.

## Running Across Multiple Server Instances

The server only works as a single instance, because connected clients and pending requests live in one process's memory. Supporting a horizontally scaled app is an open question with no plans yet. See [distributed.md](distributed.md) for a first-pass, unreviewed analysis of the options.

## Ruby on Windows

The Ruby gems are unsupported on Windows. In CI (MSYS2/UCRT, Ruby 3.1 to 3.3) the native gem's spec run exits with code 1 after about 17 examples and prints no error, which suggests a crash in the native extension (likely the GVL-release path). Diagnose it, then add `windows-latest` back to the `ruby-test` CI matrix.

## Use rustls Instead of OpenSSL

`outpunch-client` gets TLS from the system OpenSSL (`reqwest` and `tokio-tungstenite` with `native-tls`). The x86_64 Linux wheel build therefore runs `yum install openssl-devel` inside the manylinux container, and AlmaLinux's mirrors can make that take 25+ minutes (v0.5.0 dry runs: 4, 21+ and 28 minutes for the same code). Switching both to `rustls` removes the system dependency and the `yum` step, and makes every build hermetic.

## Vendor Spec for Thread#kill

`bindings/ruby/spec/vendor/ruby_gvl_spec.rb` verifies how `Thread#raise` and `Thread#wakeup` interact with
`rb_thread_call_without_gvl2`, but not `Thread#kill`; `spec/RubyGvl.tla` assumes kill behaves like raise
(the unblocking function is called and `rb_thread_check_ints` stops the call). The binding's own specs
already exercise kill (`server_spec.rb`: "can kill a thread blocked in next_outgoing", "closes the
connection when the thread running it is killed"), so a change in Ruby would still be caught, but as a
binding failure rather than a named vendor fact. Add a probe case and a claim in `vendor-claims.toml`.

## Formal Model Checks in CI

`just tla-traces` (about 15 s) and a fast subset of `spec/models.txt` should run in CI, so a change to
`crates/outpunch/src/server.rs` that the model does not allow fails the build. `just tla-all` takes
about 5 hours on all cores (most of it `RunDropOther.cfg` and `DrainDeath.cfg`), so it needs splitting into a fast CI set and a slow nightly set.

## Model Drift Check

`AxumBridge.tla` and `RubyGvl.tla` have no trace validation, so nothing notices when their code changes.
A check that maps code to models (`server.rs` → `Tunnel.tla`, `crates/outpunch-axum/src/lib.rs` →
`AxumBridge.tla`, `bindings/ruby/ext/` → `RubyGvl.tla`) and fails when a branch changes the code without
touching its model, unless the commit says why. See "Keeping models in step with the code" in
`spec/README.md`.

