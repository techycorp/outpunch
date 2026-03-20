# Outpunch — Future Work

Things explicitly deferred from phase 1. Not prioritized — just captured so nothing is lost.

## Standalone Server Binary

A standalone `outpunch-server` binary with its own HTTP + WebSocket server. The host app reverse-proxies `/tunnel/*` to it. Useful when embedding isn't an option. Also enables Docker image and Helm chart distribution.

## Multiple Clients Per Service

The server accepts multiple client connections for the same service name and round-robins between them. This is a natural consequence of deploying multiple instances with embedded clients.

**Tradeoff**: adding any form of request distribution means outpunch takes on load balancer responsibilities. Even simple round-robin raises questions: health checks? Connection weighting? Sticky sessions? Failure detection when one client disappears? Each of these grows scope significantly.

**Alternative**: keep outpunch as one client per service. If you need multiple instances, the standalone client sits in front of a real load balancer (Traefik, nginx, HAProxy) on the private side. Outpunch is the tunnel, the load balancer is the load balancer.

**Open questions**:
- If multiple embedded clients connect for the same service, does the server reject the second one, or silently accept it?
- If it accepts multiple, is simple round-robin enough, or does that create false expectations of load balancer behavior?
- Should the server even be aware of this, or should it be a strict 1:1 mapping enforced at connection time?

## Multiple Services Per Client

A single client registers for and forwards to multiple services. Already in the protocol spec (`"services": ["my-service", "pdf-gen"]`) and the Python client implementation, but not in phase 1 scope.

## Language Bindings

Client bindings use per-language FFI tools (not UniFFI — its Node.js support is experimental and Ruby support is deprecated). Python bindings via PyO3 are implemented. Remaining:

- **Node.js** (Napi-RS): mature, first-class async → Promise support, used by SWC/Rspack
- **Ruby** (Magnus + rb-sys): mature for sync, async requires manual reactor pattern (GVL release + tokio channel, proven by Temporal SDK)

Server-side adapters for other languages are written natively in each language:

- Ruby: `outpunch-rack` (Rails, Sinatra)
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
