# Reverse WebSocket Tunnel Proxy

A pattern for exposing services running on a private server through a public-facing web application, without opening any inbound ports or configuring NAT/firewall rules.

## Problem

You have a service running on a private server (behind NAT, no public IP) and a public backend. You want the frontend to be able to make HTTP requests that ultimately reach the private server, but the private server can't accept inbound connections.

## Solution: Outbound WebSocket Tunnel

The private server initiates an **outbound** WebSocket connection to the public backend. The public backend then uses this persistent connection as a bidirectional tunnel to relay HTTP requests from the frontend down to the private server and shuttle responses back.

The frontend has no awareness of the tunnel — it just calls a normal HTTP endpoint on the public backend.

## Architecture

```
Frontend (browser)
  |
  |  HTTP request (e.g., POST /tunnel/my-service/api/do-thing)
  v
Public Backend
  |
  |  Packages request into JSON payload, broadcasts over WebSocket
  v
Persistent WebSocket (initiated outbound by private server)
  |
  |  Tunnel client receives payload, forwards to local service
  v
Private Server
  |
  |  Local service processes request, returns response
  v
Tunnel Client
  |
  |  Sends response back over WebSocket
  v
Public Backend
  |
  |  Unblocks waiting HTTP handler, returns response to browser
  v
Frontend (browser)
```

## Components

There are three components to this system:

### 1. Tunnel Proxy Controller (public backend — HTTP entrypoint)

A catch-all HTTP endpoint that accepts any method and any sub-path under `/tunnel/`. When a request arrives:

1. Check if the tunnel is connected (a tunnel client is active). If not, return `502 Tunnel offline`.
2. Generate a unique `request_id` (UUID).
3. Package the incoming request into a payload:
   - `request_id`
   - HTTP `method` (GET, POST, PUT, PATCH, DELETE, etc.)
   - `path` (everything after `/tunnel/`)
   - `query` parameters
   - `headers` (filtered — see below)
   - `body` (raw request body)
4. Broadcast the payload over the WebSocket channel to the tunnel client.
5. Block the current thread on a queue/promise, waiting for the response (with a timeout, e.g., 25 seconds).
6. When the response arrives (or times out), return it to the original HTTP caller.

**Header filtering:** Strip hop-by-hop headers (`Host`, `Connection`, `Upgrade`) before forwarding. Only forward meaningful request headers (e.g., `Authorization`, `Content-Type`, custom headers).

**Error responses:**
- `502` — Tunnel offline or tunnel error
- `504` — Tunnel timeout (home server didn't respond in time)

**Route configuration:**

```
# Matches all HTTP methods, captures everything after /tunnel/ as :path
match '/tunnel/*path', to: 'tunnel#proxy', via: :all
match '/tunnel',       to: 'tunnel#proxy', via: :all
```

**Important:** This endpoint skips normal authentication/authorization since the tunnel client authenticates separately, and the proxied service may have its own auth layer.

### 2. Tunnel Proxy Service (public backend — state and coordination)

A module/service with minimal mutable state that coordinates the request/response lifecycle.

**State:**
- **Pending requests map** — a thread-safe/concurrent map of `request_id -> queue/promise`. Each in-flight request gets an entry. The controller thread blocks on the queue; the WebSocket handler pushes the response onto it.
- **Connection reference** — an atomic reference to the active WebSocket channel/connection. Used to check `connected?` status.

**Key functions:**

| Function | Purpose |
|----------|---------|
| `register_connection(channel)` | Store the active WebSocket connection. Called when tunnel client subscribes. |
| `unregister_connection(channel)` | Clear the connection reference (atomic compare-and-set so a stale disconnect doesn't clobber a new connection). Called on unsubscribe. |
| `connected?` | Returns true if a tunnel client is actively connected. |
| `send_request(payload, timeout)` | Create a queue for `request_id`, store it in the pending map, broadcast the payload over the WebSocket channel, and block on the queue with a timeout. Always clean up the pending entry in a `finally`/`ensure` block. |
| `complete_request(request_id, response_data)` | Look up the queue for `request_id` in the pending map and push the response data onto it. This unblocks the waiting controller thread. Silently ignore unknown `request_id`s (the request may have already timed out). |
| `valid_token?(token)` | Constant-time comparison of the provided token against the expected secret (from environment variable). Returns false if either is blank. |
| `extract_proxy_headers(headers)` | Filter and transform request headers for forwarding. |
| `success_response(data)` | Build a response hash from the tunnel client's data (`status`, `body`, `headers` with sensible defaults). |
| `error_response(status, message)` | Build a JSON error response. |

**Concurrency considerations:**
- The pending requests map must be thread-safe (e.g., `ConcurrentHashMap`, `Concurrent::Map`, `Map` with mutex).
- The connection reference should be atomic to handle race conditions between connect/disconnect.
- The queue/promise per request naturally handles the cross-thread handoff between the HTTP handler thread and the WebSocket handler thread.

### 3. WebSocket Channel (public backend — WebSocket endpoint)

A WebSocket channel that the tunnel client subscribes to. In the current implementation, this uses ActionCable, but the same pattern applies to any WebSocket framework (Socket.IO, ws, etc.).

**On subscribe:**
1. Validate the token parameter against the expected secret (environment variable).
2. If valid, register the connection with the proxy service and start streaming from the broadcast channel.
3. If invalid, reject the connection.

**On unsubscribe:**
- Unregister the connection from the proxy service.

**On `response` action (message from tunnel client):**
- Extract `request_id` and response data, call `complete_request(request_id, data)` on the proxy service.

**Broadcast channel name:** A fixed string (e.g., `"tunnel_channel"`). The controller broadcasts request payloads to this channel, and they're delivered to the subscribed tunnel client.

### 4. Tunnel Client (runs on private server)

A standalone script that runs on the private server. It connects **outbound** to the public backend's WebSocket endpoint and acts as a bridge to a local HTTP service.

**Lifecycle:**

1. **Connect** — Open a WebSocket to the public backend (e.g., `wss://your-app.herokuapp.com/cable`).
2. **Subscribe** — Send a subscription message for the tunnel channel with the shared secret token.
3. **Listen** — Wait for incoming messages (request payloads).
4. **Forward** — For each request payload:
   - Extract `request_id`, `method`, `path`, `query`, `headers`, `body`.
   - Make an HTTP request to the local service (e.g., `http://localhost:8080/{path}`).
   - Capture the response (`status`, `body`, `headers`).
5. **Respond** — Send the response back over the WebSocket as a `response` action with the original `request_id`.
6. **Reconnect** — On disconnect or error, wait a configurable delay (e.g., 5 seconds) and reconnect. Loop forever.

**Configuration (environment variables):**

| Variable | Purpose | Default |
|----------|---------|---------|
| `BACKEND_WS_URL` | WebSocket URL of the public backend | `ws://localhost:3000/cable` |
| `TUNNEL_SECRET` | Shared secret for authentication | (required) |
| `LOCAL_SERVER` | Base URL of the local service to forward to | `http://localhost:8080` |
| `HEARTBEAT_INTERVAL` | WebSocket ping interval in seconds | `30` |
| `RECONNECT_DELAY` | Seconds to wait before reconnecting | `5` |

## Request/Response Payload Format

**Request (backend -> tunnel client):**
```json
{
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "method": "POST",
  "path": "my-service/api/do-thing",
  "query": { "foo": "bar" },
  "headers": {
    "AUTHORIZATION": "Bearer eyJ...",
    "CONTENT-TYPE": "application/json"
  },
  "body": "{\"address\": \"123 Main St\"}"
}
```

**Response (tunnel client -> backend):**
```json
{
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": 200,
  "body": "<binary or text response>",
  "headers": {
    "Content-Type": "application/pdf",
    "Content-Disposition": "attachment; filename=\"report.pdf\""
  }
}
```

## Authentication

- The tunnel client authenticates to the backend using a **shared secret** passed as a subscription parameter, compared using constant-time string comparison.
- The secret is stored as an environment variable on both the public backend and the private server.
- The tunnel proxy endpoint itself skips the backend's normal auth middleware — the proxied service is expected to handle its own authorization if needed (e.g., Bearer tokens forwarded in headers).

## Error Handling

| Scenario | Behavior |
|----------|----------|
| No tunnel client connected | `502 Tunnel offline` returned immediately |
| Tunnel client doesn't respond in time | `504 Tunnel timeout` after 25s (configurable) |
| Local service unreachable | Tunnel client returns `502` with error message |
| Local service times out | Tunnel client returns `504` |
| WebSocket disconnects mid-request | Pending request times out naturally; client reconnects |
| Invalid/missing auth token | WebSocket subscription rejected |
| Unknown `request_id` in response | Silently ignored (request already timed out and was cleaned up) |

## Thread/Concurrency Model

The system relies on cross-thread coordination:

1. An HTTP request arrives and is handled on a **server worker thread** (e.g., Puma thread).
2. The worker thread creates a queue, stores it in the pending map keyed by `request_id`, broadcasts the payload, and **blocks** on the queue.
3. The WebSocket handler runs on a **separate thread**. When the tunnel client sends a response, the handler looks up the queue and pushes the response data.
4. The worker thread unblocks, reads the response, cleans up the pending entry, and returns the HTTP response.

This means the tunnel occupies a worker thread for the entire duration of the proxied request (up to the timeout). Plan thread pool sizing accordingly.

## Porting to a JS/Vite Backend

The current implementation uses Ruby on Rails with ActionCable (WebSocket) and Concurrent Ruby (thread-safe data structures). Here's how each piece maps to a JS backend:

### WebSocket Layer

Replace ActionCable with raw WebSocket (`ws` library) or Socket.IO:

```
ActionCable Channel  ->  ws WebSocketServer with message routing
ActionCable broadcast -> ws.send() to the registered tunnel client connection
subscribe/unsubscribe -> ws 'connection'/'close' events
response action      ->  JSON message with { type: 'response', request_id, ... }
```

With `ws`, there's no channel abstraction — you'd handle auth and message routing in `connection` and `message` event handlers directly.

### Proxy Service

The pending requests map and connection reference translate directly:

```
Concurrent::Map       ->  plain Map (JS is single-threaded, no concurrent map needed)
Concurrent::AtomicReference -> simple variable (single-threaded)
Queue (blocking)      ->  Promise with resolve/reject stored in the pending map
Timeout.timeout       ->  setTimeout that rejects the promise
```

The JS version is actually simpler because Node.js is single-threaded — no need for thread-safe data structures. Each pending request stores a `{ resolve, reject, timer }` object in a `Map<string, PendingRequest>`.

### Controller/Route

```
match '/tunnel/*path' ->  Express/Hono catch-all: app.all('/tunnel/*', handler)
request.body.read     ->  req.body (with raw body middleware)
render json:          ->  res.status(status).json(body) or res.send(body)
```

### Tunnel Client

The Python tunnel client can be rewritten in JS/Node using the `ws` library:

```
websockets (Python)   ->  ws (Node.js)
aiohttp (Python)      ->  fetch or undici
asyncio               ->  native async/await
```

Without ActionCable's protocol overhead, the WebSocket messages become simpler — just raw JSON with a `type` field to distinguish requests from responses, and a `token` field on the initial auth message.

### Simplified Protocol (without ActionCable)

```
# Auth (client -> server, first message after connect):
{ "type": "auth", "token": "shared-secret" }

# Auth response (server -> client):
{ "type": "auth_ok" }  or  close connection

# Request (server -> client):
{ "type": "request", "request_id": "...", "method": "POST", "path": "...", ... }

# Response (client -> server):
{ "type": "response", "request_id": "...", "status": 200, "body": "...", "headers": {} }

# Heartbeat: handled by ws ping/pong frames (automatic)
```

### Key Differences from ActionCable

- No `identifier` JSON-in-JSON encoding.
- No `command`/`message` wrapper — messages are flat JSON.
- No `subscribe`/`confirm_subscription` dance — auth happens on first message after connect.
- Ping/pong is handled at the WebSocket frame level (`ws` does this automatically).

## Test Coverage

The existing implementation has three levels of tests:

1. **WebSocket channel tests** — Verify subscription accepts/rejects based on token, connection registration/unregistration, and response routing to pending requests.

2. **Proxy service unit tests** — Cover token validation (matching, non-matching, nil, blank, missing env var), connection state management (register, unregister, atomic compare-and-set), request payload building, header extraction and filtering, send/receive lifecycle (broadcast + queue wait), timeout behavior, cleanup on success and timeout, and response building.

3. **Controller integration tests** — Test 502 when offline, full round-trip with simulated async response (spawn a thread/task that completes the request after a short delay), query parameter forwarding, timeout (504), error handling (502), and all HTTP methods (GET, POST, PUT, PATCH, DELETE).

4. **E2E tests (frontend)** — Cypress tests intercept the tunnel endpoint to mock various scenarios: successful responses (PDF download with Content-Disposition), HTTP errors (404, 500), network failures, and retry behavior.

## Security Considerations

- **Shared secret must be strong and kept in environment variables** — never committed to source control.
- **Constant-time comparison** for token validation to prevent timing attacks.
- **Header filtering** prevents forwarding hop-by-hop headers that could cause proxy loops or information leakage.
- **The tunnel endpoint skips normal backend auth** — ensure the proxied service validates its own authorization if needed.
- **ActionCable CSRF protection must be disabled** for the tunnel to accept external WebSocket connections (the tunnel client isn't a browser). In a JS/Vite implementation with raw `ws`, this isn't an issue since there's no CSRF layer on raw WebSockets.
- **Rate limiting** on the `/tunnel/` endpoint is advisable since it's publicly accessible and skips auth.
- **Connection hijacking** — the atomic compare-and-set on unregister prevents a stale disconnect from clobbering a freshly established connection.

---

## Appendix: An Example Ruby Implementation

This section documents a working implementation of the tunnel pattern using a **Rails 7 backend** (with ActionCable for WebSockets), a **Next.js frontend**, and a **Python tunnel client** running on the private server.

### File inventory

**Backend (Rails 7 — 9 files):**

| File | Role |
|------|------|
| `app/controllers/tunnel_controller.rb` | HTTP entrypoint — catches `/tunnel/*` requests |
| `app/services/tunnel_proxy.rb` | Coordination service — pending request map, connection state, auth |
| `app/channels/tunnel_channel.rb` | ActionCable channel — WebSocket endpoint for tunnel client |
| `config/routes.rb` | Route registration (2 lines) |
| `config/environments/production.rb` | ActionCable CSRF/origin config (2 lines) |
| `scripts/tunnel_client.py` | Python client that runs on the private server |
| `spec/channels/tunnel_channel_spec.rb` | Channel tests |
| `spec/controllers/tunnel_controller_spec.rb` | Controller tests |
| `spec/services/tunnel_proxy_spec.rb` | Proxy service tests |

**Frontend (Next.js — 2 files):**

| File | Role |
|------|------|
| Dashboard page | Single `fetch` call to `/tunnel/...` endpoint |
| E2E test | Cypress tests intercepting the tunnel endpoint |

The frontend is barely involved — it treats the tunnel endpoint as an ordinary backend API route.

### Source files

#### `app/controllers/tunnel_controller.rb`

```ruby
# frozen_string_literal: true

# TunnelController - Proxies HTTP requests through the WebSocket tunnel to home server

class TunnelController < ApplicationController
  skip_before_action :authorize

  def proxy
    return render_error(502, "Tunnel offline") unless TunnelProxy.connected?

    payload = TunnelProxy.build_request_payload(
      request_id: SecureRandom.uuid,
      method: request.method,
      path: params[:path] || "",
      query: request.query_parameters,
      headers: TunnelProxy.extract_proxy_headers(request.headers),
      body: request.body.read
    )

    response_data = TunnelProxy.send_request(payload)
    result = TunnelProxy.success_response(response_data)

    render json: result[:body], status: result[:status], headers: result[:headers]
  rescue Timeout::Error
    render_error(504, "Tunnel timeout")
  rescue => e
    Rails.logger.error("[TunnelController] Error: #{e.message}")
    render_error(502, "Tunnel error: #{e.message}")
  end

  private

  def render_error(status, message)
    result = TunnelProxy.error_response(status, message)
    render json: result[:body], status: result[:status], headers: result[:headers]
  end
end
```

#### `app/services/tunnel_proxy.rb`

```ruby
# frozen_string_literal: true

# TunnelProxy - Functional module for proxying HTTP requests through a WebSocket tunnel
#
# Minimal mutable state: just a Concurrent::Map for pending requests and a connection reference.
# All functions are stateless operations on data.

require 'concurrent'
require 'securerandom'

module TunnelProxy
  PENDING_REQUESTS = Concurrent::Map.new
  CONNECTION = Concurrent::AtomicReference.new(nil)
  DEFAULT_TIMEOUT = 25

  module_function

  # Connection management

  def register_connection(channel)
    CONNECTION.set(channel)
    Rails.logger.info("[TunnelProxy] Home server connected")
  end

  def unregister_connection(channel)
    CONNECTION.compare_and_set(channel, nil)
    Rails.logger.info("[TunnelProxy] Home server disconnected")
  end

  def connected?
    CONNECTION.get.present?
  end

  # Request handling

  def build_request_payload(request_id:, method:, path:, query:, headers:, body:)
    {
      request_id: request_id,
      method: method,
      path: path,
      query: query,
      headers: headers,
      body: body
    }
  end

  def send_request(payload, timeout: DEFAULT_TIMEOUT)
    raise "Tunnel not connected" unless connected?

    request_id = payload[:request_id]
    queue = Queue.new
    PENDING_REQUESTS[request_id] = queue

    ActionCable.server.broadcast("tunnel_channel", payload)

    begin
      Timeout.timeout(timeout) { queue.pop }
    ensure
      PENDING_REQUESTS.delete(request_id)
    end
  end

  def complete_request(request_id, response_data)
    queue = PENDING_REQUESTS[request_id]
    queue&.push(response_data)
  end

  # Authentication

  def valid_token?(token)
    expected = ENV['TUNNEL_SECRET']
    return false if expected.blank? || token.blank?
    ActiveSupport::SecurityUtils.secure_compare(token, expected)
  end

  # Header filtering

  def extract_proxy_headers(headers)
    headers
      .to_h
      .select { |k, _| k.to_s.start_with?('HTTP_') }
      .transform_keys { |k| k.to_s.sub('HTTP_', '').tr('_', '-') }
      .except('HOST', 'CONNECTION', 'UPGRADE')
  end

  # Response building

  def success_response(data)
    {
      status: data['status'] || 200,
      body: data['body'],
      headers: data['headers'] || {}
    }
  end

  def error_response(status, message)
    {
      status: status,
      body: { error: message }.to_json,
      headers: { 'Content-Type' => 'application/json' }
    }
  end
end
```

#### `app/channels/tunnel_channel.rb`

```ruby
# frozen_string_literal: true

# TunnelChannel - ActionCable channel for home server WebSocket tunnel
#
# Home server connects here with a token, subscribes to receive proxied HTTP requests,
# and sends responses back via the `response` action.

class TunnelChannel < ApplicationCable::Channel
  def subscribed
    if TunnelProxy.valid_token?(params[:token])
      stream_from "tunnel_channel"
      TunnelProxy.register_connection(self)
    else
      Rails.logger.warn("[TunnelChannel] Rejected connection: invalid token")
      reject
    end
  end

  def unsubscribed
    TunnelProxy.unregister_connection(self)
  end

  def response(data)
    TunnelProxy.complete_request(data['request_id'], data)
  end
end
```

#### `config/routes.rb` (relevant lines)

```ruby
# Tunnel proxy - forwards requests to home server via WebSocket
match '/tunnel/*path', to: 'tunnel#proxy', via: :all
match '/tunnel', to: 'tunnel#proxy', via: :all
```

#### `config/environments/production.rb` (relevant lines)

```ruby
# Action Cable configuration
config.action_cable.disable_request_forgery_protection = true
config.action_cable.allowed_request_origins = [/.*/]
```

#### `scripts/tunnel_client.py`

```python
#!/usr/bin/env python3
"""
Tunnel Client - Connects to the public backend and proxies requests to local server.

This runs on your home server. It:
1. Opens outbound WebSocket to the public backend (no incoming connections needed)
2. Stays connected, receives proxied HTTP requests
3. Forwards them to a local service
4. Sends responses back through the tunnel

Usage:
    export TUNNEL_SERVER_URL="wss://your-app.herokuapp.com/cable"
    export TUNNEL_SECRET="your-secret"
    export LOCAL_SERVER="http://localhost:8080"
    python3 tunnel_client.py

Requirements:
    pip install websockets aiohttp
"""

import asyncio
import json
import os
import signal
import sys
from typing import Optional

import aiohttp
import websockets
from websockets.exceptions import ConnectionClosed


# Config from environment
def get_config():
    return {
        "server_url": os.environ.get("TUNNEL_SERVER_URL", "ws://localhost:3000/cable"),
        "tunnel_secret": os.environ.get("TUNNEL_SECRET", "dev-secret"),
        "local_server": os.environ.get("LOCAL_SERVER", "http://localhost:8080"),
        "heartbeat_interval": int(os.environ.get("HEARTBEAT_INTERVAL", "30")),
        "reconnect_delay": int(os.environ.get("RECONNECT_DELAY", "5")),
    }


# ActionCable protocol helpers

def make_subscribe_message(token: str) -> str:
    identifier = json.dumps({"channel": "TunnelChannel", "token": token})
    return json.dumps({"command": "subscribe", "identifier": identifier})


def make_response_message(token: str, request_id: str, status: int, body: str, headers: Optional[dict] = None) -> str:
    identifier = json.dumps({"channel": "TunnelChannel", "token": token})
    data = json.dumps({
        "action": "response",
        "request_id": request_id,
        "status": status,
        "body": body,
        "headers": headers or {}
    })
    return json.dumps({"command": "message", "identifier": identifier, "data": data})


def parse_message(raw: str) -> Optional[dict]:
    """Parse ActionCable message, return inner message or None for protocol messages."""
    data = json.loads(raw)
    msg_type = data.get("type")

    if msg_type in ("welcome", "ping", "confirm_subscription"):
        return None

    if "message" in data:
        return data["message"]

    return None


# HTTP forwarding

async def forward_request(session: aiohttp.ClientSession, local_server: str, request: dict) -> dict:
    """Forward a request to the local server and return response dict."""
    url = f"{local_server}/{request.get('path', '')}"
    method = request.get("method", "GET")
    query = request.get("query", {})
    headers = request.get("headers", {})
    body = request.get("body")

    try:
        async with session.request(
            method=method,
            url=url,
            params=query,
            headers=headers,
            data=body,
            timeout=aiohttp.ClientTimeout(total=25),
            allow_redirects=False
        ) as resp:
            response_body = await resp.text()
            return {
                "status": resp.status,
                "body": response_body,
                "headers": dict(resp.headers)
            }
    except asyncio.TimeoutError:
        return {"status": 504, "body": "Local server timeout", "headers": {}}
    except aiohttp.ClientError as e:
        return {"status": 502, "body": f"Local server error: {e}", "headers": {}}


# Main connection loop

async def handle_connection(config: dict):
    """Handle a single WebSocket connection lifecycle."""
    print(f"Connecting to {config['server_url']}...")

    async with websockets.connect(
        config["server_url"],
        ping_interval=config["heartbeat_interval"],
        ping_timeout=10
    ) as ws:
        # Subscribe to tunnel channel
        await ws.send(make_subscribe_message(config["tunnel_secret"]))
        print("Connected and subscribed to TunnelChannel")

        async with aiohttp.ClientSession() as session:
            async for raw_message in ws:
                message = parse_message(raw_message)
                if message is None:
                    continue

                request_id = message.get("request_id")
                if not request_id:
                    continue

                print(f"[{request_id[:8]}] {message.get('method')} /{message.get('path', '')}")

                response = await forward_request(session, config["local_server"], message)

                print(f"[{request_id[:8]}] -> {response['status']}")

                await ws.send(make_response_message(
                    config["tunnel_secret"],
                    request_id,
                    response["status"],
                    response["body"],
                    response["headers"]
                ))


async def run_tunnel(config: dict):
    """Main loop with reconnection."""
    while True:
        try:
            await handle_connection(config)
        except ConnectionClosed as e:
            print(f"Connection closed: {e}. Reconnecting in {config['reconnect_delay']}s...")
        except Exception as e:
            print(f"Error: {e}. Reconnecting in {config['reconnect_delay']}s...")

        await asyncio.sleep(config["reconnect_delay"])


def main():
    config = get_config()

    print("Tunnel Client")
    print(f"  Server:       {config['server_url']}")
    print(f"  Local server: {config['local_server']}")
    print(f"  Heartbeat:    {config['heartbeat_interval']}s")
    print()

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)

    def shutdown(sig, frame):
        print("\nShutting down...")
        sys.exit(0)

    signal.signal(signal.SIGINT, shutdown)
    signal.signal(signal.SIGTERM, shutdown)

    loop.run_until_complete(run_tunnel(config))


if __name__ == "__main__":
    main()
```

### Test files

#### `spec/channels/tunnel_channel_spec.rb`

```ruby
require 'rails_helper'

RSpec.describe TunnelChannel, type: :channel do
  before(:each) do
    TunnelProxy::CONNECTION.set(nil)
    TunnelProxy::PENDING_REQUESTS.clear
    stub_const('ENV', { 'TUNNEL_SECRET' => 'test-secret' })
  end

  describe '#subscribed' do
    context 'with valid token' do
      it 'subscribes and registers connection' do
        subscribe(token: 'test-secret')

        expect(subscription).to be_confirmed
        expect(subscription).to have_stream_from('tunnel_channel')
        expect(TunnelProxy.connected?).to be true
      end
    end

    context 'with invalid token' do
      it 'rejects subscription' do
        subscribe(token: 'wrong-secret')

        expect(subscription).to be_rejected
        expect(TunnelProxy.connected?).to be false
      end
    end

    context 'with no token' do
      it 'rejects subscription' do
        subscribe

        expect(subscription).to be_rejected
        expect(TunnelProxy.connected?).to be false
      end
    end
  end

  describe '#unsubscribed' do
    it 'unregisters connection' do
      subscribe(token: 'test-secret')
      expect(TunnelProxy.connected?).to be true

      unsubscribe

      expect(TunnelProxy.connected?).to be false
    end
  end

  describe '#response' do
    before do
      subscribe(token: 'test-secret')
    end

    it 'completes pending request with response data' do
      queue = Queue.new
      TunnelProxy::PENDING_REQUESTS['req-123'] = queue

      perform :response, { 'request_id' => 'req-123', 'status' => 200, 'body' => 'success' }

      result = queue.pop(true) rescue nil
      expect(result['request_id']).to eq('req-123')
      expect(result['status']).to eq(200)
      expect(result['body']).to eq('success')
    end

    it 'handles unknown request_id gracefully' do
      expect {
        perform :response, { 'request_id' => 'unknown', 'status' => 200 }
      }.not_to raise_error
    end
  end
end
```

#### `spec/controllers/tunnel_controller_spec.rb`

```ruby
require 'rails_helper'

RSpec.describe TunnelController, type: :controller do
  before(:each) do
    TunnelProxy::CONNECTION.set(nil)
    TunnelProxy::PENDING_REQUESTS.clear
  end

  describe 'POST #proxy' do
    let(:path) { 'api/test' }

    context 'when tunnel is not connected' do
      it 'returns 502 with error message' do
        post :proxy, params: { path: path }

        expect(response.status).to eq(502)
        expect(JSON.parse(response.body)).to eq({ 'error' => 'Tunnel offline' })
      end
    end

    context 'when tunnel is connected' do
      let(:channel) { double('channel') }

      before do
        TunnelProxy.register_connection(channel)
      end

      it 'sends request through tunnel and returns response' do
        expect(ActionCable.server).to receive(:broadcast) do |channel_name, payload|
          expect(channel_name).to eq('tunnel_channel')
          expect(payload[:method]).to eq('POST')
          expect(payload[:path]).to eq('api/test')
          expect(payload[:request_id]).to be_present

          # Simulate async response
          Thread.new do
            sleep 0.05
            TunnelProxy.complete_request(payload[:request_id], {
              'status' => 200,
              'body' => '{"result": "success"}',
              'headers' => {}
            })
          end
        end

        post :proxy, params: { path: path }

        expect(response.status).to eq(200)
        expect(response.body).to eq('{"result": "success"}')
      end

      it 'forwards query parameters' do
        expect(ActionCable.server).to receive(:broadcast) do |_, payload|
          expect(payload[:query]).to eq({ 'foo' => 'bar' })

          Thread.new do
            sleep 0.05
            TunnelProxy.complete_request(payload[:request_id], { 'status' => 200, 'body' => 'ok' })
          end
        end

        get :proxy, params: { path: path, foo: 'bar' }

        expect(response.status).to eq(200)
      end

      it 'returns 504 on timeout' do
        allow(TunnelProxy).to receive(:send_request).and_raise(Timeout::Error)

        post :proxy, params: { path: path }

        expect(response.status).to eq(504)
        expect(JSON.parse(response.body)).to eq({ 'error' => 'Tunnel timeout' })
      end

      it 'returns 502 on other errors' do
        allow(TunnelProxy).to receive(:send_request).and_raise(StandardError.new('Connection lost'))

        post :proxy, params: { path: path }

        expect(response.status).to eq(502)
        expect(JSON.parse(response.body)['error']).to include('Connection lost')
      end
    end
  end

  describe 'different HTTP methods' do
    let(:channel) { double('channel') }

    before do
      TunnelProxy.register_connection(channel)
    end

    %w[GET POST PUT PATCH DELETE].each do |http_method|
      it "handles #{http_method} requests" do
        expect(ActionCable.server).to receive(:broadcast) do |_, payload|
          expect(payload[:method]).to eq(http_method)

          Thread.new do
            sleep 0.05
            TunnelProxy.complete_request(payload[:request_id], { 'status' => 200, 'body' => 'ok' })
          end
        end

        process :proxy, method: http_method.downcase.to_sym, params: { path: 'test' }

        expect(response.status).to eq(200)
      end
    end
  end
end
```

#### `spec/services/tunnel_proxy_spec.rb`

```ruby
require 'rails_helper'

RSpec.describe TunnelProxy do
  before(:each) do
    # Reset state between tests
    TunnelProxy::CONNECTION.set(nil)
    TunnelProxy::PENDING_REQUESTS.clear
  end

  describe '.valid_token?' do
    before do
      stub_const('ENV', { 'TUNNEL_SECRET' => 'test-secret' })
    end

    it 'returns true for matching token' do
      expect(TunnelProxy.valid_token?('test-secret')).to be true
    end

    it 'returns false for non-matching token' do
      expect(TunnelProxy.valid_token?('wrong-secret')).to be false
    end

    it 'returns false for nil token' do
      expect(TunnelProxy.valid_token?(nil)).to be false
    end

    it 'returns false for blank token' do
      expect(TunnelProxy.valid_token?('')).to be false
    end

    context 'when TUNNEL_SECRET is not set' do
      before do
        stub_const('ENV', {})
      end

      it 'returns false' do
        expect(TunnelProxy.valid_token?('any-token')).to be false
      end
    end
  end

  describe '.register_connection / .unregister_connection / .connected?' do
    let(:channel) { double('channel') }

    it 'starts disconnected' do
      expect(TunnelProxy.connected?).to be false
    end

    it 'is connected after registration' do
      TunnelProxy.register_connection(channel)
      expect(TunnelProxy.connected?).to be true
    end

    it 'is disconnected after unregistration' do
      TunnelProxy.register_connection(channel)
      TunnelProxy.unregister_connection(channel)
      expect(TunnelProxy.connected?).to be false
    end

    it 'does not unregister a different channel' do
      other_channel = double('other_channel')
      TunnelProxy.register_connection(channel)
      TunnelProxy.unregister_connection(other_channel)
      expect(TunnelProxy.connected?).to be true
    end
  end

  describe '.build_request_payload' do
    it 'builds a payload hash with all fields' do
      payload = TunnelProxy.build_request_payload(
        request_id: 'abc-123',
        method: 'POST',
        path: 'api/test',
        query: { foo: 'bar' },
        headers: { 'X-Custom' => 'value' },
        body: '{"data": 1}'
      )

      expect(payload).to eq({
        request_id: 'abc-123',
        method: 'POST',
        path: 'api/test',
        query: { foo: 'bar' },
        headers: { 'X-Custom' => 'value' },
        body: '{"data": 1}'
      })
    end
  end

  describe '.extract_proxy_headers' do
    it 'extracts HTTP_ prefixed headers and transforms keys' do
      headers = {
        'HTTP_X_CUSTOM_HEADER' => 'value1',
        'HTTP_AUTHORIZATION' => 'Bearer token',
        'HTTP_HOST' => 'example.com',
        'HTTP_CONNECTION' => 'keep-alive',
        'CONTENT_TYPE' => 'application/json'
      }

      result = TunnelProxy.extract_proxy_headers(headers)

      expect(result).to eq({
        'X-CUSTOM-HEADER' => 'value1',
        'AUTHORIZATION' => 'Bearer token'
      })
      expect(result).not_to have_key('HOST')
      expect(result).not_to have_key('CONNECTION')
      expect(result).not_to have_key('CONTENT_TYPE')
    end
  end

  describe '.send_request' do
    let(:channel) { double('channel') }
    let(:payload) { { request_id: 'test-id', method: 'GET', path: 'test' } }

    before do
      TunnelProxy.register_connection(channel)
    end

    it 'raises error when not connected' do
      TunnelProxy.unregister_connection(channel)

      expect {
        TunnelProxy.send_request(payload, timeout: 1)
      }.to raise_error(RuntimeError, 'Tunnel not connected')
    end

    it 'broadcasts to tunnel_channel and waits for response' do
      expect(ActionCable.server).to receive(:broadcast).with('tunnel_channel', payload)

      # Simulate async response in another thread
      Thread.new do
        sleep 0.1
        TunnelProxy.complete_request('test-id', { 'status' => 200, 'body' => 'ok' })
      end

      result = TunnelProxy.send_request(payload, timeout: 2)
      expect(result).to eq({ 'status' => 200, 'body' => 'ok' })
    end

    it 'times out if no response received' do
      expect(ActionCable.server).to receive(:broadcast).with('tunnel_channel', payload)

      expect {
        TunnelProxy.send_request(payload, timeout: 0.1)
      }.to raise_error(Timeout::Error)
    end

    it 'cleans up pending request after completion' do
      expect(ActionCable.server).to receive(:broadcast)

      Thread.new do
        sleep 0.05
        TunnelProxy.complete_request('test-id', { 'status' => 200 })
      end

      TunnelProxy.send_request(payload, timeout: 1)
      expect(TunnelProxy::PENDING_REQUESTS['test-id']).to be_nil
    end

    it 'cleans up pending request after timeout' do
      expect(ActionCable.server).to receive(:broadcast)

      expect {
        TunnelProxy.send_request(payload, timeout: 0.1)
      }.to raise_error(Timeout::Error)

      expect(TunnelProxy::PENDING_REQUESTS['test-id']).to be_nil
    end
  end

  describe '.complete_request' do
    it 'does nothing if request_id not found' do
      expect {
        TunnelProxy.complete_request('nonexistent', { 'status' => 200 })
      }.not_to raise_error
    end
  end

  describe '.success_response' do
    it 'builds response with defaults' do
      result = TunnelProxy.success_response({ 'body' => 'hello' })

      expect(result).to eq({
        status: 200,
        body: 'hello',
        headers: {}
      })
    end

    it 'uses provided status and headers' do
      result = TunnelProxy.success_response({
        'status' => 201,
        'body' => 'created',
        'headers' => { 'X-Custom' => 'val' }
      })

      expect(result).to eq({
        status: 201,
        body: 'created',
        headers: { 'X-Custom' => 'val' }
      })
    end
  end

  describe '.error_response' do
    it 'builds error response with JSON body' do
      result = TunnelProxy.error_response(502, 'Gateway error')

      expect(result[:status]).to eq(502)
      expect(JSON.parse(result[:body])).to eq({ 'error' => 'Gateway error' })
      expect(result[:headers]).to eq({ 'Content-Type' => 'application/json' })
    end
  end
end
```

### Frontend touchpoint

The frontend's only interaction is a single `fetch` call in the dashboard component. It has no awareness of the tunnel — it's just hitting a backend URL:

```typescript
const apiUrl = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:3000/api'
const token = localStorage.getItem('my-service_auth_token')
const response = await fetch(`${apiUrl}/tunnel/my-service/api/generate-report`, {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    ...(token && { 'Authorization': `Bearer ${token}` }),
  },
  body: JSON.stringify({
    address,
    target_date: targetDate
  })
})
```

The E2E tests intercept this endpoint with `cy.intercept('POST', '**/tunnel/my-service/api/generate-report', ...)` to mock success, HTTP errors (404, 500), network failures, loading states, and retry behavior.
