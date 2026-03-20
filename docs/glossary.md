# Outpunch — Glossary

## Core

The `outpunch` library crate. Contains all tunnel logic: protocol types, message serialization, pending request tracking, connection state, authentication. Has zero web framework dependencies and zero WebSocket library dependencies — it works entirely with its own plain types (`TunnelRequest`, `TunnelResponse`, etc. built from `String`, `HashMap`, `u16`) and communicates through tokio message channels.

## Server Framework Adapter

A thin translation layer between a web framework and the core. The adapter handles three things:

1. **Tunnel endpoint** (`/tunnel/*path`) — translates the framework's HTTP request type into a `TunnelRequest`, calls the core, translates the `TunnelResponse` back into the framework's HTTP response type.
2. **WebSocket upgrade** (`/ws`) — uses the framework's HTTP upgrade mechanism to establish a WebSocket connection.
3. **WebSocket bridge** — a small loop that pipes messages between the WebSocket stream and the core's message channels.

The core never touches a WebSocket type. The adapter bridges the WebSocket I/O to the core's channels. This is what makes outpunch multi-language — any language's WebSocket library can feed messages into the same channel interface.

Each adapter is small and specific to one framework. Examples: `outpunch-axum` (Rust/axum), `outpunch-rack` (Ruby), `outpunch-asgi` (Python), `outpunch-express` (Node.js).

## Tunnel Client

The outpunch client binary. Runs on the private network, connects outbound via WebSocket to the server, receives proxied HTTP requests, forwards them to local services, and sends responses back. No framework compatibility concerns — it's always a standalone process.

## Tunnel Request

A plain data struct representing an HTTP request to be proxied through the tunnel. Contains: `request_id`, `service`, `method`, `path`, `query`, `headers`, `body`. Used as both the core's internal type and the JSON message sent over the WebSocket.

## Tunnel Response

A plain data struct representing the response from the proxied service. Contains: `request_id`, `status`, `headers`, `body`, `body_encoding`. Used as both the core's internal type and the JSON message sent back over the WebSocket.

## Pending Request Map

The core's central coordination structure. Maps `request_id` to a waiting HTTP handler. When an HTTP request enters the tunnel, the core parks a handler in the map and sends the request over the WebSocket. When a response arrives, the core matches the `request_id` to the waiting handler and delivers the response. Handles timeouts, cleanup, and concurrent access. This is the primary reason the core exists as a shared Rust component.

## Standalone Mode

Outpunch runs as its own process with its own HTTP + WebSocket server. The host app reverse-proxies `/tunnel/*` to it. Framework-agnostic — works with any stack.

## Embedded Mode

The host app imports an adapter crate and mounts outpunch's routes into its own server. Single process, no extra port. Requires an adapter for the host app's framework.
