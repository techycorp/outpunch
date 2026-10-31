# Changelog

## v0.5.5

### Fixed

- **Ruby: `outpunch-ruby` is installable on every platform again.** 0.5.4 published only the
  `x86_64-linux` gem, because the release's registry check took that gem as proof the source gem
  existed too. On macOS and arm64 Linux, `outpunch-ruby` 0.5.4 and the Rack and Rails gems that pin
  it can't be installed; use 0.5.5.
- **Ruby: the Rack and Rails adapters load with the precompiled gem.** On `x86_64-linux`, 0.5.4's
  adapters failed at boot with `cannot load such file -- outpunch/outpunch`.

### Changed

- **The `outpunch-ruby` source gem ships its `Cargo.lock`,** so a compile on install uses the
  dependency versions we tested.
- **npm Linux binaries need glibc 2.17** instead of 2.34, so they load on older distributions.
- **README:** Node.js is listed as a server adapter only; the npm package has no tunnel client.
- **Package metadata:** the Ruby gems have a homepage, and the Python package has a license and
  project URLs.

## v0.5.4

### Added

- **Ruby: `outpunch-ruby` ships precompiled for `x86_64-linux`** (Ruby 3.1 to 4.0), so installing it
  there needs no Rust toolchain. Bundler projects need `bundle lock --add-platform x86_64-linux`.
  Other platforms still compile from the source gem.

### Changed

- **The client uses rustls instead of OpenSSL on every platform and binding.** Certificates are
  still verified against the system trust store, so custom and corporate CAs keep working. TLS 1.0
  and 1.1 servers are refused, and TLS error messages have changed.

## v0.5.3

### Added

- **Ruby: a `before_proxy` hook can stop a request with `Outpunch::HookHalt`.** Raise
  `Outpunch::HookHalt.new(status, body)` from the hook and the tunnel answers with that status
  and JSON body without forwarding anything to the service (for example a 402 when a user may
  not use it). Any other error a hook raises still answers 502, as before.

## v0.5.2

### Fixed

- **Ruby: the native gem no longer drops messages, skips calls or closes connections when a
  thread is interrupted without an exception.** Ruby's `rb_thread_call_without_gvl2` does not run
  the native function at all while an interrupt is pending, and the VM timer makes one pending
  whenever threads compete for the GVL (routine in a multi-threaded Puma server). The gem treated
  that as a completed call, so `push_message` could silently drop a client's message (including
  its `auth`, leaving the client without `auth_ok`), `run` could return without running the
  connection, and `handle_request` could fail with `tunnel request interrupted`. `Thread#wakeup`
  also closed a running connection and failed waiting requests. Calls now retry until they run;
  only an exception (`Thread#raise`, `Thread#kill`, `Timeout`) stops them. This was the
  intermittent `auth_ok` failure in the Rack specs on Linux (8 of 25 suite runs in a 2-CPU Linux
  container before the fix, 0 of 25 after).

## v0.5.1

No package changes. npm now publishes through trusted publishing, like crates.io, PyPI and
RubyGems; this release is the first npm publish to use it.

## v0.5.0

Every package now shares one version, the release tag's. Versions between a package's last
release and 0.5.0 were skipped.

| Package | Registry | Version |
|---|---|---|
| `outpunch`, `outpunch-axum`, `outpunch-client` | crates.io | 0.5.0 |
| `outpunch` | npm | 0.5.0 (first publish) |
| `outpunch` | PyPI | 0.5.0 |
| `outpunch-ruby`, `outpunch-ruby-rack`, `outpunch-ruby-rails` | RubyGems | 0.5.0 (first publish under these names) |

### Breaking changes

- **Ruby gems are renamed:** `outpunch` → `outpunch-ruby`, `outpunch-rack` → `outpunch-ruby-rack`,
  `outpunch-rails` → `outpunch-ruby-rails`. Due to a clerical error we lost access to the RubyGems
  account that owns the original names. We created those gems; they are orphaned at 0.1.x. Remove
  them before installing the new names, since both provide `require "outpunch"`. If we recover the
  account, we'll move back to the original names. `require` paths and module names are unchanged.
- **Ruby: `outpunch-ruby-rack` requires `outpunch-ruby` at exactly its own version.**

### Fixed

- **npm: `import { outpunch } from "outpunch/vite"` works.** The package now ships compiled
  JavaScript with an `exports` map for `outpunch` and `outpunch/vite`; it previously shipped raw
  TypeScript with no `outpunch/vite` entry point. The release installs the packed package into an
  empty project and loads both entry points through `require` and `import` before publishing.

### Changed

- **One version for every package.** `python scripts/version.py bump <version>` sets it everywhere;
  the release fails if any package disagrees with the tag. See [docs/releasing.md](docs/releasing.md).
- **Releases fail on any publish error.** Previously every publish error was ignored, so v0.4.0
  reported success while npm and RubyGems publishes failed.
- **Releases publish through trusted publishing** on crates.io, PyPI and RubyGems; no long-lived
  registry tokens for those.
- **Repository moved** to `github.com/techycorp/outpunch`; package metadata now points there.
- The Ruby gems are not supported on Windows; CI no longer runs the Ruby tests on Windows.

### Release notes for v0.4.0

The v0.4.0 release published only to crates.io and PyPI. The npm and RubyGems publishes failed
and the release workflow ignored the errors.

## v0.4.0

| Package | Registry | Version |
|---|---|---|
| `outpunch`, `outpunch-axum`, `outpunch-client` | crates.io | 0.2.0 |
| `outpunch` | npm | 0.2.0 (not published, see Unreleased) |
| `outpunch` | PyPI | 0.2.2 |
| `outpunch` (native gem), `outpunch-rack`, `outpunch-rails` | RubyGems | 0.2.0 (not published, see Unreleased) |

### Breaking changes

- **Rust: `ServerConfig` has a new public field, `auth_timeout`.** Code that builds
  `ServerConfig` with a struct literal must add the field or end with
  `..ServerConfig::default()`.
- **Ruby: `outpunch-rack` and `outpunch-rails` now run on the Rust core** through the
  native `outpunch` gem (0.2.0), which is a new runtime dependency. The public Ruby API is
  unchanged. Query and header values are converted to strings, and `Outpunch::Rack::Server`
  raises `ArgumentError` when the secret is blank. Installing the gem compiles a Rust
  extension, so a Rust toolchain (`cargo`) must be available at install time.
- **Ruby: `outpunch-rails` no longer ships `lib/outpunch.rb`.** Apps that call
  `require "outpunch"` to load the Rails adapter should require `outpunch/rails` instead.
  Bundler's auto-require is unaffected.
- **A server with an empty secret rejects every client.** Previously, a client sending an
  empty token was accepted.

### Fixed

- A client reconnecting before the server noticed its old socket had closed could take the
  tunnel offline.
- Requests in flight when a client disconnected waited for the full timeout (504) instead of
  failing immediately (502).
- One client that stopped reading could stall requests to every service, and the send to that
  client ignored the request timeout.
- The axum adapter silently dropped requests when a client read slowly.
- The client handled one request at a time, so a slow request delayed all others and paused
  keepalive pings.
- Connections that never sent an auth message were held open forever. They are now closed after
  `auth_timeout` (default 10s).
- Any authenticated client could answer a request sent to a different service.
- The native `outpunch` gem can now be installed from RubyGems. Earlier versions referenced
  Rust crates outside the gem and failed to compile on install.
- The Node adapter now closes the WebSocket when the server ends a connection (failed auth,
  auth timeout, or replaced by a newer client).

### Changed

- When a second client authenticates for a service that already has one, the newer client
  takes over and the older connection is closed.

### Added

- Rust: `OutpunchServer::try_handle_request`, which returns tunnel failures as a typed
  `TunnelError` instead of an error response.
- Ruby: `Outpunch::Server` and `Outpunch::Connection` in the native gem, with
  `Outpunch::NotConnectedError`, `Outpunch::TunnelTimeoutError` (a `Timeout::Error`) and
  `Outpunch::TunnelError`.
