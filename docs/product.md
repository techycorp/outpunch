# Outpunch — Product Overview

Outpunch is a reverse tunnel proxy. It lets you expose services running on a private network through a public-facing server, without opening inbound ports or configuring NAT/firewall rules.

## The Problem

You have a service on a private machine (behind NAT, no public IP). You have a public web app. You want the web app to make HTTP requests that reach the private service. The private machine can't accept inbound connections.

## How Outpunch Solves It

The private machine runs the **outpunch client**, which opens an outbound WebSocket connection to the **outpunch server** running on (or embedded in) the public app. The server uses this persistent connection as a bidirectional tunnel to relay HTTP requests to the private service and return responses.

The public app's frontend has no awareness of the tunnel — it hits a normal HTTP endpoint.

```
Browser / API consumer
  |
  |  HTTP request: POST /tunnel/my-service/api/do-thing
  v
Public App (with outpunch server embedded, or proxying to standalone outpunch)
  |
  |  Packages request, sends over WebSocket
  v
Outpunch Client (on private network, connected outbound)
  |
  |  Forwards to local service
  v
Local Service (e.g., http://localhost:8080)
  |
  |  Response flows back through the tunnel
  v
Browser / API consumer
```

## Components

### Outpunch Server

The server-side component. Responsibilities:

- Accept and authenticate WebSocket connections from outpunch clients
- Expose an HTTP endpoint (e.g., `/tunnel/:service_name/*path`) that captures incoming requests
- Package HTTP requests as JSON, send them through the WebSocket to the appropriate client
- Block until the client responds (with a configurable timeout), then return the response to the original HTTP caller
- Track pending requests and match responses by `request_id`

**Deployment modes:**

1. **Embedded** — imported as a library into your web app. You mount it as middleware or a route handler. This is the primary use case: outpunch runs in the same process as your web server.
2. **Standalone** — runs as its own process with its own HTTP + WebSocket server. Your app reverse-proxies tunnel routes to it. Useful when you can't or don't want to embed it.

### Outpunch Client

The client-side component. Runs on the private machine. Responsibilities:

- Connect outbound via WebSocket to the outpunch server
- Authenticate with a shared secret
- Subscribe to one or more named services
- Receive proxied HTTP requests, forward them to the configured local URL
- Send responses back through the WebSocket
- Reconnect automatically on disconnect
- Support hot-reloading service configuration without restart

**Deployment:** standalone binary. Configure via environment variables and a services config file.

## Multi-Service Routing

A single WebSocket connection multiplexes multiple services. The service name is part of the URL path:

```
POST /tunnel/my-service/api/report    → forwarded to my-service's local URL
POST /tunnel/pdf-gen/convert          → forwarded to pdf-gen's local URL
```

The client declares which services it handles via configuration:

```yaml
services:
  my-service:
    url: http://localhost:8080
  pdf-gen:
    url: http://localhost:9000
```

The server routes each request to the client subscription matching the service name.

## Protocol

Outpunch uses a simple raw WebSocket protocol. No framework-specific abstractions (no ActionCable, no Socket.IO).

### Handshake

```
Client connects via WebSocket
Client → Server:  {"type": "auth", "token": "shared-secret", "services": ["my-service", "pdf-gen"]}
Server → Client:  {"type": "auth_ok"}
  or server closes the connection on auth failure
```

### Request/Response

```
Server → Client:  {
  "type": "request",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "service": "my-service",
  "method": "POST",
  "path": "api/do-thing",
  "query": {"foo": "bar"},
  "headers": {"Authorization": "Bearer eyJ...", "Content-Type": "application/json"},
  "body": "..."
}

Client → Server:  {
  "type": "response",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": 200,
  "body": "...",
  "headers": {"Content-Type": "application/pdf"},
  "body_encoding": "base64"
}
```

### Heartbeat

Handled at the WebSocket frame level (ping/pong). No application-level ping messages.

## Authentication Model

Layered:

1. **Tunnel client ↔ server** — outpunch authenticates tunnel clients via a shared secret during the WebSocket handshake. This is outpunch's responsibility.
2. **End user ↔ public app** — the host application authenticates end users before requests reach the tunnel endpoint. This is the host app's responsibility. Outpunch doesn't know about users.
3. **Forwarded headers** — outpunch passes through request headers (e.g., `Authorization`) so the local service behind the tunnel can perform its own validation if needed.

## Implementation

### Language

Rust. The core logic (protocol, connection management, request/response lifecycle, HTTP forwarding) is implemented once in Rust.

### Works With Any Web Framework

The core library has **zero web framework dependencies** and **zero WebSocket library dependencies**. It works entirely with its own plain types (strings, maps, integers) and communicates through message channels. This means outpunch is not tied to any specific HTTP server or WebSocket library — in any language.

To embed outpunch in a web app, you use a **server framework adapter** — a thin layer that translates between the framework's types and outpunch's types. The adapter handles:

1. **Tunnel endpoint** — translate the framework's HTTP request/response to/from outpunch types
2. **WebSocket upgrade** — use the framework's upgrade mechanism to establish a connection
3. **WebSocket bridge** — pipe messages between the WebSocket stream and the core's message channels

The core never touches a WebSocket type directly. The adapter bridges WebSocket I/O into the core's channels. This is what makes outpunch truly multi-language — any language's WebSocket library can feed messages into the same channel interface.

This applies at every level:

- **Rust**: adapter crates per framework (axum first, then actix-web, etc.)
- **Ruby**: adapter for Rack (covers Rails, Sinatra, etc.)
- **Python**: adapter for ASGI (covers Django, FastAPI) or WSGI (covers Flask)
- **Node.js**: adapter per framework (Express, Hono, Fastify)

Standalone mode (outpunch as its own process, host app reverse-proxies to it) is always available as a framework-agnostic fallback. See [architecture.md](architecture.md) for implementation details.

### Bindings

Language bindings (Python, Ruby, Node.js) are a future goal, built via UniFFI or similar. Each binding is a thin wrapper around the Rust core — not a reimplementation.

The standalone Rust server and client ship first. Bindings come later.

## Open Questions

- **Binary body encoding**: the current implementation base64-encodes response bodies. Should the protocol support binary WebSocket frames instead?
- **Service authorization**: should outpunch enforce which services a given client is allowed to register for, or is the shared secret sufficient?
- **Multiple clients**: can multiple clients connect for the same service (load balancing)? Or is it one client per service?
- **TLS/mTLS**: should outpunch support mutual TLS for client authentication as an alternative to shared secrets?
