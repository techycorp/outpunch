# Changelog

## Unreleased

### Breaking changes

- **Rust: `ServerConfig` has a new public field, `auth_timeout`.** Code that builds
  `ServerConfig` with a struct literal must add the field or end with
  `..ServerConfig::default()`.
- **Ruby: `outpunch-rack` and `outpunch-rails` now run on the Rust core** through the
  native `outpunch` gem (0.2.0), which is a new runtime dependency. The public Ruby API is
  unchanged. Query and header values are converted to strings, and `Outpunch::Rack::Server`
  raises `ArgumentError` when the secret is blank.
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
