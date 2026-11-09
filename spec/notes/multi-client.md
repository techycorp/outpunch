# Design Notes: Multiple Clients per Service

Status: design notes, not decided, not built. Input for a second TLA+ spec after the current-behavior
spec is checked.

## Why

Target deployment: the outpunch server on Heroku, two or more clients for the same service in different
regions, reachable only over the public internet. There is no shared private network, so the alternative
in `docs/future.md:15` (one client per service behind a private-side load balancer) does not fit.

Today these clients would displace each other every few seconds (newest wins, `docs/future.md:17`).

## Proposed shape

- **Clients coexist** per service; the server picks one per request (round-robin or failover; open).
- **Replacement keyed by client instance, not by service.** Each client process generates a random
  instance ID at startup, held only in memory and reused across that process's reconnects. A connection
  presenting an ID that is already connected replaces that ID's old socket immediately, which keeps
  today's fast-reconnect behavior for a client whose old socket died unnoticed. No user configuration.
- **Dead-socket eviction.** An instance that dies and never returns leaves a socket that must be evicted
  (ping timeout or send failure). Until then requests may be routed to it; this is the new scope that
  `docs/future.md:13` warns about (health checks, failure detection).

## Rule: the instance ID is a hint, never a correctness input

We cannot assume clients send it correctly. Every misuse may cost latency or 502s, never a wrong answer:

| Instance ID behavior | Effect | Cost |
|---|---|---|
| Correct | Reconnect replaces its own dead socket instantly | None |
| Missing (older client) | Treated as a new instance; coexists; a dead socket lingers until evicted | Slow or failed requests until eviction |
| New on every reconnect | Same as missing | Same |
| Same ID on two live clients | They replace each other (today's flapping); server logs a warning (`docs/future.md:73`) | Churn and 502s |
| Spoofed by a holder of the shared secret | Can displace another client | No new risk: the shared secret already allows registering for any service |

The second spec models the instance ID adversarially (any client may send any ID or none, at any time)
and checks that the safety requirements (exactly one outcome per request, only the target connection can
answer, no leaks) hold in every case.

## Related options

- **Graceful drain on replacement:** the replaced connection stops receiving new requests but may answer
  the ones it already has, then closes when they finish or time out. Standard practice (HTTP/2 GOAWAY,
  RFC 9113 §6.8; load balancer deregistration delay). Only useful when the replaced connection is alive;
  draining a dead socket turns a fast 502 into a slow 504. With instance IDs, "same ID" means likely dead
  (fail fast) and "different ID" no longer replaces at all.
- **Safe rerouting:** a request that was queued for a dead socket but never written to it cannot have been
  applied, so it may be resent to another client. A request already written may not be resent unless it
  is known to be idempotent (RFC 9110 §9.2.2).

## Separate constraint: multiple server dynos

All of the above assumes one server process. With several Heroku dynos, a request can land on a dyno that
holds none of the service's sockets. See `docs/distributed.md`.

## Open questions

1. Selection policy: round-robin, failover (primary/standby), or region-aware?
2. Eviction: ping interval and timeout values; what happens to requests in flight on an evicted socket.
3. Backward compatibility: how a server treats clients that send no instance ID, and how older servers
   treat clients that do.

## What the TLA+ model showed

`spec/MultiClient.tla` models this design (instance ID adversarial: none, distinct, duplicated or spoofed;
any selection policy; silent socket death; ping-timeout eviction). Results:

- Safety holds under every ID behavior: no wrong answer (a response only from the connection the request
  was sent to), no leaked pending entries, no finished connection left registered. Replacement happens only
  between connections presenting the same non-None ID; distinct IDs never displace each other. A duplicated
  (or spoofed) ID can displace a live client, so flapping remains possible only in that case.
- A request gets NotConnected only when no open, authenticated connection is registered for the service
  (apart from one being replaced, or one not yet registered).
- Every request ends; a dead socket is eventually closed and unregistered, but only because eviction is
  fair. Without a ping timeout a dead socket stays registered forever and keeps attracting requests.
- Cost of silent death: a request can be routed to a dead socket while a live one is registered. It fails
  (502 once eviction closes the connection, or 504 at the request timeout), never hangs. The ping timeout
  should be shorter than the request timeout, or these become 504s.
- Regression for clients that send no ID: today, newest-wins replaces a dead socket as soon as the old
  client reconnects. In this design a no-ID reconnect coexists with its own dead socket until eviction, and
  requests can be routed to the dead one. Options: keep newest-wins for connections without an ID (legacy
  mode, two old live clients still flap), or require the ID.
- Model bounds: the full message alphabet with 2 messages per connection did not finish in 10+ minutes;
  the checked instances drop malformed messages and wrong tokens (covered by Tunnel.tla) and limit the
  3-connection run to auth messages only.
