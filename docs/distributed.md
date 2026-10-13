# Outpunch — Running Across Multiple Server Instances

> **Status: open question, not reviewed.** This is a first-pass analysis that nobody has
> critiqued yet. There are no plans to build any of it. It exists to record what we know so far.

## The Question

Outpunch currently works only when the public app runs as a single instance. Can it support a
horizontally scaled, distributed app?

Getting out of NAT and running many server instances are separate problems. Nothing about the
outbound WebSocket prevents a distributed setup. The limit is where the server keeps its state.

## What Blocks It Today

The core keeps two tables in the memory of one process:

1. **Connected services:** which services have a client connected to this instance, and the
   WebSocket to reach each one.
2. **Pending requests:** which requests are waiting, and which caller to wake when each response
   arrives.

Pending requests already fit a distributed setup: a waiting request lives on the instance that
received the HTTP call, and its response only has to get back to that instance.

Connected services is the real problem. A request can land on an instance that doesn't hold the
client's WebSocket, and that instance returns 502. Everything decided from that table is also local
to one instance today: whether a service is connected, which client wins when two connect, and
closing the replaced connection.

## Possible Approaches

| Option | How | Rough cost | Main drawback |
|---|---|---|---|
| **A. One tunnel hub** | Run outpunch as its own single process (the standalone server in [future.md](future.md)). Every app instance forwards `/tunnel/*` to it. | Small | The app is distributed but the hub isn't. If the hub dies, the tunnel is down. |
| **B. Client connects to every instance** | The tunnel client opens one WebSocket per app instance, so every instance has its own connection. No coordination between instances. | Medium | The client must reach each instance directly, which a load balancer prevents. Needs multiple clients per service. |
| **C. Message broker relay** | The instance that receives a request publishes it to a broker (Redis, NATS). The instance holding the WebSocket forwards it and publishes the response back to the asking instance. | Large | New failure modes: broker outage, an instance dying mid-request, large bodies through the broker. Plain pub/sub delivers to every subscriber, so it needs a registry of which instance owns each service, or a queue that guarantees a single consumer. |
| **D. Route requests to the right instance** | The load balancer sends `/tunnel/<service>` to the instance holding that service's WebSocket. | Depends on the load balancer | Most load balancers can't route on "which instance holds this connection", so this needs a custom routing layer. |

## Observations

- The core already talks to the outside world only through plain types and channels, so a seam for
  "send this request to whichever instance owns service X" would be a contained change, not a
  rewrite.
- A broker client in the Rust core would give Python, Node and Ruby distributed mode through one
  configuration setting, with no per-language work.
- If this is ever needed, option A is the cheapest first step. Option C is the general answer if a
  single hub becomes a real problem.

## Open Questions

- Is there real demand for this, or is single-instance the right scope for outpunch?
- Is option A enough for most people who run more than one instance?
- For option C, which broker, and what delivery guarantees are needed?
- How should "newest client wins" and connection replacement behave across instances?
