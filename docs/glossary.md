# Outpunch — Glossary

## Core

The `outpunch` library crate. Contains all tunnel logic: protocol types, message serialization, pending request tracking, connection state, authentication. Has zero web framework dependencies — it works entirely with its own plain types (`TunnelRequest`, `TunnelResponse`, etc. built from `String`, `HashMap`, `u16`).

## Server Framework Adapter

A thin translation layer between a web framework and the core. The adapter handles only HTTP-level concerns:

1. **Tunnel endpoint** (`/tunnel/*path`) — translates the framework's HTTP request type into a `TunnelRequest`, calls the core, translates the `TunnelResponse` back into the framework's HTTP response type.
2. **WebSocket upgrade** (`/ws`) — uses the framework's HTTP upgrade mechanism to establish a WebSocket connection, then hands the raw WebSocket stream to the core.

After the WebSocket upgrade, the adapter is no longer involved. The core owns the WebSocket connection entirely — authentication, message routing, heartbeat, everything. The adapter only touches HTTP.

Each adapter is small (~20-50 lines) and specific to one framework. Examples: `outpunch-axum` (Rust/axum), `outpunch-rack` (Ruby), `outpunch-asgi` (Python), `outpunch-express` (Node.js).

## Tunnel Client

The outpunch client binary. Runs on the private network, connects outbound via WebSocket to the server, receives proxied HTTP requests, forwards them to local services, and sends responses back. No framework compatibility concerns — it's always a standalone process.

## Tunnel Request

A plain data struct representing an HTTP request to be proxied through the tunnel. Contains: `request_id`, `service`, `method`, `path`, `query`, `headers`, `body`. Used as both the core's internal type and the JSON message sent over the WebSocket.

## Tunnel Response

A plain data struct representing the response from the proxied service. Contains: `request_id`, `status`, `headers`, `body`, `body_encoding`. Used as both the core's internal type and the JSON message sent back over the WebSocket.

## Standalone Mode

Outpunch runs as its own process with its own HTTP + WebSocket server. The host app reverse-proxies `/tunnel/*` to it. Framework-agnostic — works with any stack.

## Embedded Mode

The host app imports an adapter crate and mounts outpunch's routes into its own server. Single process, no extra port. Requires an adapter for the host app's framework.
