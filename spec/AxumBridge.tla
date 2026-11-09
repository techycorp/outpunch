----------------------------- MODULE AxumBridge -----------------------------
(***************************************************************************)
(* One tunnel connection through the axum adapter                           *)
(* (crates/outpunch-axum/src/lib.rs, handle_ws) and the core's outgoing     *)
(* path (crates/outpunch/src/server.rs: dispatch's send, the drain task and *)
(* the on_message callback). Requirements: spec/REQUIREMENTS.md, R26-R28.   *)
(*                                                                         *)
(* Message path for a request r:                                           *)
(*   dispatch: outgoing_tx.send(r)   core channel, bounded (OutCap)         *)
(*   drain task: callback(r)         msg_tx.send(r), unbounded, never waits *)
(*   write task: msg_rx.recv, then   drops r if its dispatch has ended      *)
(*     (is_pending), else ws_sink.send(r): waits while the peer is stalled  *)
(*                                                                         *)
(* Lifetimes:                                                              *)
(*   - The drain task ends when every outgoing sender is gone: run()'s own  *)
(*     (dropped after cleanup), the services-map handle (dropped at cleanup *)
(*     or when replaced), and one per dispatch still in progress.           *)
(*   - Ending the drain task drops the callback, hence msg_tx; the write    *)
(*     task then ends once its queue is drained, or when a send fails.      *)
(*   - handle_ws: after run() returns, waits up to 100 ms for the write     *)
(*     task, then aborts it and the read task (F6 fix).                     *)
(*   - A socket write that takes longer than the request timeout closes   *)
(*     the connection and ends the write task (F7 fix).                     *)
(*                                                                         *)
(* The client is authenticated at the start; auth is in Tunnel.tla.        *)
(* Responses come back through the incoming path, abstracted to Answer.    *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    Reqs,          \* request ids
    OutCap,        \* capacity of the core's outgoing channel (64 in the code)
    PeerResolves,  \* TRUE: a stalled peer eventually reads again or its socket errors
    WriteTimeouts  \* TRUE: a write blocked longer than the request timeout closes the connection (F7)

PeerStates == {"reading", "stalled", "gone"}
ReqStates == {"idle", "sending", "waiting", "done"}

VARIABLES
    req,        \* Reqs -> state of the dispatch for r
    outq,       \* core outgoing channel: Seq(Reqs)
    wq,         \* adapter msg channel (unbounded): Seq(Reqs)
    written,    \* requests written to the socket (the peer may read them any time after)
    late,       \* requests written after their dispatch had already ended
    held,       \* the message the write task has taken and is writing: {} or {r}
    excused,    \* requests whose dispatch ended while the write task held them
    peer,       \* the remote client's behavior
    core,       \* run(): "relay" | "drain_wait" | "done"
    open,       \* the connection is still open (incoming_tx present)
    registered, \* the services map holds this connection's handle
    drain,      \* the drain task is running
    write,      \* the write task is running
    ws          \* handle_ws: "running" | "flush" | "returned"

vars == <<req, outq, wq, written, late, held, excused, peer, core, open, registered, drain, write, ws>>

TypeOK ==
    /\ req \in [Reqs -> ReqStates]
    /\ \A i \in 1..Len(outq) : outq[i] \in Reqs
    /\ \A i \in 1..Len(wq) : wq[i] \in Reqs
    /\ written \subseteq Reqs /\ late \subseteq Reqs
    /\ held \subseteq Reqs /\ Cardinality(held) <= 1 /\ excused \subseteq Reqs
    /\ peer \in PeerStates
    /\ core \in {"relay", "drain_wait", "done"}
    /\ open \in BOOLEAN /\ registered \in BOOLEAN
    /\ drain \in BOOLEAN /\ write \in BOOLEAN
    /\ ws \in {"running", "flush", "returned"}

Init ==
    /\ req = [r \in Reqs |-> "idle"]
    /\ outq = <<>> /\ wq = <<>>
    /\ written = {} /\ late = {}
    /\ held = {} /\ excused = {}
    /\ peer = "reading"
    /\ core = "relay"
    /\ open = TRUE /\ registered = TRUE
    /\ drain = TRUE /\ write = TRUE
    /\ ws = "running"

InSeq(s, x) == \E i \in 1..Len(s) : s[i] = x

(***************************************************************************)
(* Environment: the peer, HTTP callers, replacement                         *)
(***************************************************************************)

Stall == peer = "reading" /\ peer' = "stalled" /\ UNCHANGED <<req, outq, wq, written, late, held, excused, core, open, registered, drain, write, ws>>
Unstall == peer = "stalled" /\ peer' = "reading" /\ UNCHANGED <<req, outq, wq, written, late, held, excused, core, open, registered, drain, write, ws>>
\* The socket errors (TCP reset, or a stalled peer's connection finally fails).
PeerGone == peer /= "gone" /\ peer' = "gone" /\ UNCHANGED <<req, outq, wq, written, late, held, excused, core, open, registered, drain, write, ws>>

\* The peer sends Close or its socket ends: the read task calls close().
PeerClose ==
    /\ open /\ peer /= "reading"
    /\ open' = FALSE
    /\ UNCHANGED <<req, outq, wq, written, late, held, excused, peer, core, registered, drain, write, ws>>

\* A newer client registers for the service: our handle leaves the map and we are closed.
Replaced ==
    /\ registered
    /\ registered' = FALSE /\ open' = FALSE
    /\ UNCHANGED <<req, outq, wq, written, late, held, excused, peer, core, drain, write, ws>>

\* dispatch: route to this connection while it is registered.
Start(r) ==
    /\ req[r] = "idle" /\ registered
    /\ req' = [req EXCEPT ![r] = "sending"]
    /\ UNCHANGED <<outq, wq, written, late, held, excused, peer, core, open, registered, drain, write, ws>>

\* outgoing_tx.send completes (channel has room; the receiver is alive while we hold a sender).
\* After cleanup the pending entry is gone, so the dispatch then fails with Disconnected.
CoreSend(r) ==
    /\ req[r] = "sending" /\ Len(outq) < OutCap
    /\ outq' = Append(outq, r)
    /\ req' = [req EXCEPT ![r] = IF core = "relay" THEN "waiting" ELSE "done"]
    /\ UNCHANGED <<wq, written, late, held, excused, peer, core, open, registered, drain, write, ws>>

\* The request timer fires (also cancels a send still waiting for room: tokio's
\* Sender::send is cancel safe, the message is not sent). PendingGuard cleans up.
Timeout(r) ==
    /\ req[r] \in {"sending", "waiting"}
    /\ req' = [req EXCEPT ![r] = "done"]
    /\ excused' = excused \cup (held \cap {r})
    /\ UNCHANGED <<outq, wq, written, late, held, peer, core, open, registered, drain, write, ws>>

\* The client answered and the relay loop delivered it.
Answer(r) ==
    /\ req[r] = "waiting" /\ r \in written /\ core = "relay"
    /\ req' = [req EXCEPT ![r] = "done"]
    /\ UNCHANGED <<outq, wq, written, late, held, excused, peer, core, open, registered, drain, write, ws>>

(***************************************************************************)
(* Server: core run(), drain task, write task, handle_ws                    *)
(***************************************************************************)

\* Relay loop sees the channel closed; cleanup unregisters and fails pending
\* requests (they end via Disconnected), then run() drops its sender and
\* awaits the drain task.
RelayEnd ==
    /\ core = "relay" /\ ~open
    /\ core' = "drain_wait"
    /\ registered' = FALSE
    /\ req' = [r \in Reqs |-> IF req[r] = "waiting" THEN "done" ELSE req[r]]
    /\ excused' = excused \cup {r \in held : req[r] = "waiting"}
    /\ UNCHANGED <<outq, wq, written, late, held, peer, open, drain, write, ws>>

\* Drain task: deliver one message to the callback (msg_tx.send). If the write
\* task has ended, msg_tx.send fails and the message is discarded.
DrainStep ==
    /\ drain /\ outq /= <<>>
    /\ outq' = Tail(outq)
    /\ wq' = IF write THEN Append(wq, Head(outq)) ELSE wq
    /\ UNCHANGED <<req, written, late, held, excused, peer, core, open, registered, drain, write, ws>>

\* Drain task ends: channel empty and no sender left.
\* A dispatch holds its sender clone until it returns.
SendersLeft == core = "relay" \/ registered \/ \E r \in Reqs : req[r] \in {"sending", "waiting"}
DrainEnd ==
    /\ drain /\ outq = <<>> /\ ~SendersLeft
    /\ drain' = FALSE
    /\ UNCHANGED <<req, outq, wq, written, late, held, excused, peer, core, open, registered, write, ws>>

RunReturns ==
    /\ core = "drain_wait" /\ ~drain
    /\ core' = "done" /\ ws' = "flush"
    /\ UNCHANGED <<req, outq, wq, written, late, held, excused, peer, open, registered, drain, write>>

\* Write task: msg_rx.recv takes the next message; is_pending drops it if its
\* dispatch has already ended.
WriteTake ==
    /\ write /\ held = {} /\ wq /= <<>>
    /\ wq' = Tail(wq)
    /\ held' = IF req[Head(wq)] = "done" THEN {} ELSE {Head(wq)}
    /\ UNCHANGED <<req, outq, written, late, excused, peer, core, open, registered, drain, write, ws>>

\* Write task: ws_sink.send of the held message. Waits while the peer is stalled; the
\* dispatch can end meanwhile, and the message is still written.
WriteSend ==
    /\ write /\ held /= {} /\ peer /= "stalled"
    /\ held' = {}
    /\ IF peer = "reading"
       THEN /\ written' = written \cup held
            /\ late' = late \cup {r \in held : req[r] = "done"}
            /\ UNCHANGED write
       ELSE /\ write' = FALSE                     \* send error: the task breaks
            /\ UNCHANGED <<written, late>>
    /\ UNCHANGED <<req, outq, wq, excused, peer, core, open, registered, drain, ws>>

\* The write timeout fires while the write waits on a stalled peer: the
\* write task closes the connection and ends (F7 fix).
WriteTimeout ==
    /\ WriteTimeouts
    /\ write /\ held /= {} /\ peer = "stalled"
    /\ write' = FALSE /\ held' = {} /\ open' = FALSE
    /\ UNCHANGED <<req, outq, wq, written, late, excused, peer, core, registered, drain, ws>>

\* Write task ends: msg_tx is gone (drain ended) and the queue is empty.
WriteEnd ==
    /\ write /\ wq = <<>> /\ held = {} /\ ~drain
    /\ write' = FALSE
    /\ UNCHANGED <<req, outq, wq, written, late, held, excused, peer, core, open, registered, drain, ws>>

\* handle_ws returns: the write task finished, or the 100 ms flush timer fired
\* (modeled as possible at any time) and the task is aborted (F6 fix).
WsReturns ==
    /\ ws = "flush"
    /\ ws' = "returned"
    /\ write' = FALSE /\ held' = {}
    /\ UNCHANGED <<req, outq, wq, written, late, excused, peer, core, open, registered, drain>>

Next ==
    \/ Stall \/ Unstall \/ PeerGone \/ PeerClose \/ Replaced
    \/ \E r \in Reqs : Start(r) \/ CoreSend(r) \/ Timeout(r) \/ Answer(r)
    \/ RelayEnd \/ DrainStep \/ DrainEnd \/ RunReturns \/ WriteTake \/ WriteSend \/ WriteTimeout \/ WriteEnd \/ WsReturns

Fairness ==
    /\ WF_vars(RelayEnd) /\ WF_vars(DrainStep) /\ WF_vars(DrainEnd) /\ WF_vars(RunReturns)
    /\ WF_vars(WriteTake) /\ WF_vars(WriteTimeout) /\ WF_vars(WriteEnd) /\ WF_vars(WsReturns)
    \* The write task runs whenever the peer reads, even if it keeps stalling in between.
    /\ SF_vars(WriteSend)
    /\ \A r \in Reqs : WF_vars(Timeout(r))

\* With PeerResolves, a stalled peer does not stay stalled forever.
PeerFairness == PeerResolves => (WF_vars(Unstall) \/ WF_vars(PeerGone))

Spec == Init /\ [][Next]_vars /\ Fairness /\ PeerFairness

(***************************************************************************)
(* Properties                                                              *)
(***************************************************************************)

\* R26: the adapter queues only messages for requests still in progress (memory per
\* connection is bounded by the requests in flight, not by everything ever sent).
QueueOnlyLive == \A i \in 1..Len(wq) : req[wq[i]] /= "done"

\* R27: a request is not written to the socket after its caller got an answer, except the
\* one the write task was already writing when the caller was answered (a write to a
\* stalled socket waits). A request written before can still be read later; only the
\* protocol could close that window.
NoLateWrite == late \subseteq excused

\* The stricter form, which the in-flight message breaks (F5 residual): must be VIOLATED.
NoLateWriteAtAll == late = {}

\* R28: once the connection is over, the write task (and with it the socket) ends.
WriteTaskEnds == (core = "done") ~> ~write

\* R29: a client that stays connected but stops reading is disconnected: a write blocked on
\* it ends with the connection unregistered, unless the peer reads again first.
StalledPeerUnregistered == (held /= {} /\ peer = "stalled") ~> (~registered \/ peer /= "stalled")

\* Every request ends (the bridge cannot make one hang).
EveryRequestEnds == \A r \in Reqs : req[r] \in {"sending", "waiting"} ~> req[r] = "done"

\* Witnesses (must be VIOLATED).
NeverWritten == written = {}
NeverDrainEnds == drain
NeverWriteEnds == write
\* Only WriteTimeout ends the write task while the peer is stalled and run() is relaying.
NeverWriteTimeout == ~(~write /\ peer = "stalled" /\ core = "relay")

=============================================================================
