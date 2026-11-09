------------------------------- MODULE Tunnel -------------------------------
(***************************************************************************)
(* Model of the outpunch server core: crates/outpunch/src/server.rs.       *)
(* Requirements are numbered as in spec/REQUIREMENTS.md (R1..R23).          *)
(*                                                                         *)
(* Granularity: every section of server.rs that holds the server mutex is   *)
(* one atomic action; every .await outside the mutex is a point where other *)
(* actions may interleave.                                                 *)
(*                                                                         *)
(* Environment (no fairness, may do anything at any time):                 *)
(*   - tunnel clients: connect, push any message, close, or stall;          *)
(*     clients may collude, so a response may name any request id that was  *)
(*     sent to any connection;                                             *)
(*   - HTTP callers: start requests, and may be cancelled (the future is    *)
(*     dropped, as when an HTTP client disconnects or a binding aborts the  *)
(*     task).                                                              *)
(*                                                                         *)
(* Abstractions:                                                           *)
(*   - Request ids are distinct model values (UUIDs, R11).                 *)
(*   - The outgoing channel and drain task are abstracted to one step,     *)
(*     Send, which may never happen (a client that stops reading).         *)
(*   - Timers are nondeterministic: a timeout may fire whenever its        *)
(*     future is pending.                                                  *)
(*   - Channel capacities (64) are not modeled.                            *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets, TLC

CONSTANTS
    Conns,      \* connection ids
    Services,   \* service names
    Reqs,       \* request ids
    SecretSet,  \* TRUE: server has a non-empty secret (R2 checks FALSE)
    MaxMsgs,    \* bound on messages each connection may push
    Reconnects, \* TRUE: a finished connection id may be reused by a new connection
    RunDrops,   \* TRUE: the run() future may be dropped at an await (F3)
    DrainDies,  \* TRUE: the outgoing drain task may die, e.g. a panicking callback (F4)
    None        \* model value

Tokens == {"right", "wrong"}

Msgs ==
    [type : {"auth"}, svc : Services, token : Tokens]
        \cup [type : {"resp"}, req : Reqs]
        \cup [type : {"junk"}]

Phases == {"idle", "created", "auth", "reg", "kick", "relay", "cleanup", "done", "dropped"}
CallerStates == {"idle", "sending", "waiting", "removing", "dropping", "done", "cancelled"}

VARIABLES
    services,   \* Services -> Conns \cup {None}: services map
    pending,    \* Reqs -> Conns \cup {None}: pending map, value = owning conn_id
    conn,       \* Conns -> per-connection state (run() and its incoming channel)
    caller,     \* Reqs -> state of the dispatch() future for that request
    oneshot,    \* Reqs -> state of the response oneshot channel
    delivered,  \* set of request ids sent to some client (what clients can name)
    authed,     \* ghost: Conns -> token presented in the auth that registered it, or None
    lastReg     \* ghost: Services -> last connection to register, or None

vars == <<services, pending, conn, caller, oneshot, delivered, authed, lastReg>>
\* run() is no longer running (returned, or its future was dropped).
Finished(c) == conn[c].phase \in {"done", "dropped"}

TypeOK ==
    /\ services \in [Services -> Conns \cup {None}]
    /\ pending \in [Reqs -> Conns \cup {None}]
    /\ \A c \in Conns :
        /\ conn[c].phase \in Phases
        /\ conn[c].open \in BOOLEAN
        /\ Len(conn[c].inbox) <= MaxMsgs
        /\ \A i \in 1..Len(conn[c].inbox) : conn[c].inbox[i] \in Msgs
        /\ conn[c].svc \in Services \cup {None}
        /\ conn[c].old \in Conns \cup {None}
        /\ conn[c].pushed \in 0..MaxMsgs
        /\ conn[c].drain \in BOOLEAN
    /\ \A r \in Reqs :
        /\ caller[r].st \in CallerStates
        /\ caller[r].conn \in Conns \cup {None}
        /\ caller[r].res \in {"none", "ok", "502", "504"}
        /\ caller[r].from \in Conns \cup {None}
        /\ caller[r].spurious \in BOOLEAN
        /\ caller[r].late504 \in BOOLEAN
        /\ caller[r].beatValue \in BOOLEAN
    /\ \A r \in Reqs :
        /\ oneshot[r].st \in {"empty", "value", "closed"}
        /\ oneshot[r].from \in Conns \cup {None}
    /\ delivered \subseteq Reqs
    /\ authed \in [Conns -> Tokens \cup {None}]
    /\ lastReg \in [Services -> Conns \cup {None}]

Init ==
    /\ services = [s \in Services |-> None]
    /\ pending = [r \in Reqs |-> None]
    /\ conn = [c \in Conns |->
                [phase |-> "idle", open |-> FALSE, inbox |-> <<>>,
                 svc |-> None, old |-> None, pushed |-> 0, drain |-> TRUE]]
    /\ caller = [r \in Reqs |-> [st |-> "idle", conn |-> None, res |-> "none", from |-> None,
                                  spurious |-> FALSE, late504 |-> FALSE,
                                  beatValue |-> FALSE]]
    /\ oneshot = [r \in Reqs |-> [st |-> "empty", from |-> None]]
    /\ delivered = {}
    /\ authed = [c \in Conns |-> None]
    /\ lastReg = [s \in Services |-> None]

\* Dropping a pending entry drops its oneshot sender: an empty oneshot becomes closed.
DropSender(os, rs) ==
    [r \in Reqs |-> IF r \in rs /\ os[r].st = "empty"
                        THEN [st |-> "closed", from |-> None]
                        ELSE os[r]]

(***************************************************************************)
(* Environment: tunnel clients and the adapter                             *)
(***************************************************************************)

\* create_connection(): the incoming channel exists; run() has not started.
Connect(c) ==
    /\ conn[c].phase = "idle"
    /\ conn' = [conn EXCEPT ![c].phase = "created", ![c].open = TRUE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* The adapter calls run(). Messages pushed (and close() called) before this
\* are buffered in the channel, as the API allows.
RunStart(c) ==
    /\ conn[c].phase = "created"
    /\ conn' = [conn EXCEPT ![c].phase = "auth"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* push_message(): only reaches run() while incoming_tx is present.
Push(c, m) ==
    /\ conn[c].open
    /\ conn[c].phase \notin {"idle", "done", "dropped"}
    /\ conn[c].pushed < MaxMsgs
    /\ m.type = "resp" => m.req \in delivered
    /\ conn' = [conn EXCEPT ![c].inbox = Append(@, m), ![c].pushed = @ + 1]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* close(), or the socket dies: incoming_tx is taken.
Close(c) ==
    /\ conn[c].open
    /\ conn' = [conn EXCEPT ![c].open = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* With Reconnects, a finished connection's id is reused for a new connection
\* (models a client that reconnects forever with finitely many ids). Only
\* allowed once nothing refers to the old one, so reuse is indistinguishable
\* from a fresh conn_id.
Reconnect(c) ==
    /\ Reconnects
    /\ conn[c].phase = "done"
    /\ \A s \in Services : services[s] /= c
    /\ \A x \in Conns : ~(conn[x].phase = "kick" /\ conn[x].old = c)
    /\ \A r \in Reqs : pending[r] /= c /\ (caller[r].conn = c => caller[r].st \in {"idle", "done", "cancelled"})
    /\ conn' = [conn EXCEPT ![c] = [phase |-> "idle", open |-> FALSE, inbox |-> <<>>,
                                    svc |-> None, old |-> None, pushed |-> 0, drain |-> TRUE]]
    /\ authed' = [authed EXCEPT ![c] = None]
    /\ lastReg' = [s \in Services |-> IF lastReg[s] = c THEN None ELSE lastReg[s]]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered>>

(***************************************************************************)
(* Connection::run()                                                        *)
(***************************************************************************)

Valid(m) == m.type = "auth" /\ SecretSet /\ m.token = "right"

\* handle_auth(): first message must be a valid auth (R1, R2, R3, R5).
Auth(c) ==
    /\ conn[c].phase = "auth"
    /\ conn[c].inbox /= <<>>
    /\ LET m == Head(conn[c].inbox) IN
        IF Valid(m)
        THEN /\ conn' = [conn EXCEPT ![c].inbox = Tail(@), ![c].phase = "reg", ![c].svc = m.svc]
             /\ authed' = [authed EXCEPT ![c] = m.token]
        ELSE /\ conn' = [conn EXCEPT ![c].inbox = Tail(@), ![c].phase = "done"]
             /\ UNCHANGED authed
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, lastReg>>

\* handle_auth(): the auth timer fires while no message is buffered (R4),
\* or recv() returns None because the channel closed.
AuthTimeout(c) ==
    /\ conn[c].phase = "auth"
    /\ conn[c].inbox = <<>>
    /\ conn' = [conn EXCEPT ![c].phase = "done"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* Register under the lock; newest wins (R9).
Register(c) ==
    /\ conn[c].phase = "reg"
    /\ LET s == conn[c].svc IN
        /\ services' = [services EXCEPT ![s] = c]
        /\ lastReg' = [lastReg EXCEPT ![s] = c]
        /\ conn' = [conn EXCEPT ![c].phase = "kick", ![c].old = services[s]]
    /\ UNCHANGED <<pending, caller, oneshot, delivered, authed>>

\* Outside the lock: close the replaced connection by taking its incoming_tx.
Kick(c) ==
    /\ conn[c].phase = "kick"
    /\ LET o == conn[c].old IN
        conn' = IF o = None
                THEN [conn EXCEPT ![c].phase = "relay"]
                ELSE [conn EXCEPT ![c].phase = "relay", ![o].open = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* Relay loop: one message, under the lock for responses. Only the owning
\* connection may complete a request (R12, R13); anything else is ignored
\* (R18, R22).
Relay(c) ==
    /\ conn[c].phase = "relay"
    /\ conn[c].inbox /= <<>>
    /\ LET m == Head(conn[c].inbox) IN
        IF m.type = "resp" /\ pending[m.req] = c
        THEN /\ pending' = [pending EXCEPT ![m.req] = None]
             /\ oneshot' = [oneshot EXCEPT ![m.req] = [st |-> "value", from |-> c]]
        ELSE UNCHANGED <<pending, oneshot>>
    /\ conn' = [conn EXCEPT ![c].inbox = Tail(@)]
    /\ UNCHANGED <<services, caller, delivered, authed, lastReg>>

\* recv() returns None: channel closed and drained.
RelayEnd(c) ==
    /\ conn[c].phase = "relay"
    /\ conn[c].inbox = <<>>
    /\ ~conn[c].open
    /\ conn' = [conn EXCEPT ![c].phase = "cleanup"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* cleanup_connection() under the lock: unregister only if still ours (R10);
\* fail this connection's pending requests (R17).
Cleanup(c) ==
    /\ conn[c].phase = "cleanup"
    /\ LET s == conn[c].svc
           mine == {r \in Reqs : pending[r] = c}
       IN
        /\ services' = IF services[s] = c THEN [services EXCEPT ![s] = None] ELSE services
        /\ pending' = [r \in Reqs |-> IF r \in mine THEN None ELSE pending[r]]
        /\ oneshot' = DropSender(oneshot, mine)
    /\ conn' = [conn EXCEPT ![c].phase = "done"]
    /\ UNCHANGED <<caller, delivered, authed, lastReg>>

\* The run() future is dropped while it awaits (auth or the relay loop): nothing
\* after the await runs. Once registered, run() holds a guard whose destructor runs
\* cleanup_connection, which takes the lock, so Cleanup is a separate step (F3 fix).
RunDropped(c) ==
    /\ RunDrops
    /\ conn[c].phase \in {"auth", "relay"}
    /\ conn' = [conn EXCEPT ![c].phase = IF conn[c].phase = "relay" THEN "cleanup" ELSE "dropped"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* The drain task dies (the on_message callback panicked): sends to this
\* connection's outgoing channel now fail.
DrainDeath(c) ==
    /\ DrainDies
    /\ conn[c].phase \notin {"idle", "done", "dropped"}
    /\ conn[c].drain
    /\ conn' = [conn EXCEPT ![c].drain = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

\* The drain task's CloseOnDrop guard runs while the task unwinds: close() takes
\* incoming_tx, so the relay loop ends and run() cleans up (F4 fix).
DrainClose(c) ==
    /\ ~conn[c].drain /\ conn[c].open
    /\ conn' = [conn EXCEPT ![c].open = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, lastReg>>

(***************************************************************************)
(* OutpunchServer::dispatch() for request r                                 *)
(***************************************************************************)

\* Under the lock: look up the service; register pending before sending (R7, R8).
Start(r, s) ==
    /\ caller[r].st = "idle"
    /\ IF services[s] = None
       THEN /\ caller' = [caller EXCEPT ![r].st = "done", ![r].res = "502",
                            ![r].spurious = lastReg[s] /= None /\ conn[lastReg[s]].phase /= "done"]
            /\ UNCHANGED pending
       ELSE /\ pending' = [pending EXCEPT ![r] = services[s]]
            /\ caller' = [caller EXCEPT ![r].st = "sending", ![r].conn = services[s]]
    /\ UNCHANGED <<services, conn, oneshot, delivered, authed, lastReg>>

\* outgoing_tx.send() completes and the drain task hands it to the client.
\* No fairness: a client that stops reading blocks this forever (R16).
Send(r) ==
    /\ caller[r].st = "sending"
    /\ conn[caller[r].conn].drain
    /\ caller' = [caller EXCEPT ![r].st = "waiting"]
    /\ delivered' = delivered \cup {r}
    /\ UNCHANGED <<services, pending, conn, oneshot, authed, lastReg>>

\* outgoing_tx.send() fails because the drain task's receiver is gone (ConnectionLost).
SendFail(r) ==
    /\ caller[r].st = "sending"
    /\ ~conn[caller[r].conn].drain
    /\ caller' = [caller EXCEPT ![r].st = "removing", ![r].res = "502"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, lastReg>>

\* resp_rx resolves: a response, or the sender was dropped (Disconnected).
Receive(r) ==
    /\ caller[r].st = "waiting"
    /\ oneshot[r].st /= "empty"
    /\ caller' = IF oneshot[r].st = "value"
                 THEN [caller EXCEPT ![r].st = "done", ![r].res = "ok", ![r].from = oneshot[r].from]
                 ELSE [caller EXCEPT ![r].st = "removing", ![r].res = "502"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, lastReg>>

\* The request timer fires during send or wait (R15, R16). tokio's Timeout polls
\* the inner future first, so a resolved oneshot always wins over the timer.
Timeout(r) ==
    /\ caller[r].st \in {"sending", "waiting"}
    /\ ~(caller[r].st = "waiting" /\ oneshot[r].st /= "empty")
    /\ caller' = [caller EXCEPT ![r].st = "removing", ![r].res = "504",
                    ![r].late504 = caller[r].st = "waiting" /\ Finished(caller[r].conn),
                    ![r].beatValue = caller[r].st = "waiting" /\ oneshot[r].st = "value"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, lastReg>>

\* On error, dispatch returns; dropping PendingGuard removes the entry under the lock (R19).
Remove(r) ==
    /\ caller[r].st = "removing"
    /\ pending' = [pending EXCEPT ![r] = None]
    /\ oneshot' = DropSender(oneshot, {r})
    /\ caller' = [caller EXCEPT ![r].st = "done"]
    /\ UNCHANGED <<services, conn, delivered, authed, lastReg>>

\* The dispatch future is dropped while it awaits. Code after the current
\* .await never runs; PendingGuard's destructor then takes the lock (other
\* steps may run first) and removes the entry (R19; fix for F1).
Cancel(r) ==
    /\ caller[r].st \in {"sending", "waiting"}
    /\ caller' = [caller EXCEPT ![r].st = "dropping"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, lastReg>>

GuardDrop(r) ==
    /\ caller[r].st = "dropping"
    /\ pending' = [pending EXCEPT ![r] = None]
    /\ oneshot' = DropSender(oneshot, {r})
    /\ caller' = [caller EXCEPT ![r].st = "cancelled"]
    /\ UNCHANGED <<services, conn, delivered, authed, lastReg>>

(***************************************************************************)
(* Specification                                                           *)
(***************************************************************************)

Environment ==
    \/ \E c \in Conns : Connect(c) \/ RunStart(c) \/ Close(c) \/ Reconnect(c) \/ RunDropped(c) \/ DrainDeath(c)
    \/ \E c \in Conns, m \in Msgs : Push(c, m)
    \/ \E r \in Reqs, s \in Services : Start(r, s)
    \/ \E r \in Reqs : Send(r) \/ Cancel(r)

Server ==
    \/ \E c \in Conns :
        Auth(c) \/ AuthTimeout(c) \/ Register(c) \/ Kick(c)
            \/ Relay(c) \/ RelayEnd(c) \/ Cleanup(c) \/ DrainClose(c)
    \/ \E r \in Reqs : Receive(r) \/ Timeout(r) \/ Remove(r) \/ SendFail(r) \/ GuardDrop(r)

Next == Environment \/ Server

\* Server-side steps are scheduled fairly; the environment is not.
Fairness ==
    /\ \A c \in Conns :
        /\ WF_vars(Auth(c)) /\ WF_vars(AuthTimeout(c)) /\ WF_vars(Register(c))
        /\ WF_vars(Kick(c)) /\ WF_vars(Relay(c)) /\ WF_vars(RelayEnd(c))
        /\ WF_vars(Cleanup(c)) /\ WF_vars(DrainClose(c))
    /\ \A r \in Reqs :
        /\ WF_vars(Receive(r)) /\ WF_vars(Timeout(r)) /\ WF_vars(Remove(r))
        /\ WF_vars(SendFail(r)) /\ WF_vars(GuardDrop(r))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* Safety properties                                                       *)
(***************************************************************************)

\* R1: a service only routes to a connection that presented the right secret for it.
RoutedOnlyIfAuthed ==
    \A s \in Services :
        services[s] /= None =>
            /\ SecretSet /\ authed[services[s]] = "right"
            /\ conn[services[s]].svc = s

\* R2: an empty secret rejects every client.
EmptySecretRejectsAll == ~SecretSet => \A c \in Conns : authed[c] = None

\* R9: once a newer connection has taken over, the one it replaced is closed.
\* (Only meaningful without connection-id reuse.)
ReplacedIsClosed ==
    ~Reconnects =>
        \A c \in Conns :
            conn[c].phase \in {"relay", "cleanup", "done", "dropped"} /\ conn[c].old /= None
                => ~conn[conn[c].old].open

\* R17: a request waiting on a connection that has finished is not answered 504.
NoTimeoutAfterDisconnect == \A r \in Reqs : ~caller[r].late504

\* R15: the timer never wins over a response that has already arrived.
NoTimeoutOverResponse == \A r \in Reqs : ~caller[r].beatValue

\* R10: while the last connection to register is still running, the service
\* still points at it; only its own cleanup removes it.
NoStaleRemoval ==
    \A s \in Services :
        lastReg[s] /= None /\ conn[lastReg[s]].phase /= "done" => services[s] = lastReg[s]

\* R12, R13: a successful response came from the connection the request was sent to.
ResponseFromTarget ==
    \A r \in Reqs : caller[r].res = "ok" => caller[r].from = caller[r].conn

\* R13: a pending entry is owned by the connection its request was routed to.
PendingOwnedByTarget ==
    \A r \in Reqs : pending[r] /= None => pending[r] = caller[r].conn

\* R17, R19, R24: a connection whose run() is over owns no pending requests.
NoPendingOnFinishedConn ==
    \A r \in Reqs : pending[r] /= None => ~Finished(pending[r])

\* R21: while the newest registrant for a service is still running, no request
\* to that service is answered NotConnected.
NoSpuriousNotConnected == \A r \in Reqs : ~caller[r].spurious

\* R19: a cancelled request leaves no pending entry.
NoPendingAfterCancel ==
    \A r \in Reqs : caller[r].st = "cancelled" => pending[r] = None

Safety ==
    /\ RoutedOnlyIfAuthed
    /\ EmptySecretRejectsAll
    /\ NoStaleRemoval
    /\ ResponseFromTarget
    /\ PendingOwnedByTarget
    /\ NoPendingOnFinishedConn
    /\ NoPendingAfterCancel
    /\ NoSpuriousNotConnected
    /\ ReplacedIsClosed
    /\ NoTimeoutAfterDisconnect
    /\ NoTimeoutOverResponse

(***************************************************************************)
(* Liveness properties                                                     *)
(***************************************************************************)

InFlight(r) == caller[r].st \in {"sending", "waiting", "removing", "dropping"}

\* R14: every started request ends (completes or its caller goes away).
EveryRequestEnds == \A r \in Reqs : InFlight(r) ~> caller[r].st \in {"done", "cancelled"}

\* R19: a pending entry whose caller is gone (or going) is eventually removed.
NoLeak ==
    \A r \in Reqs :
        (pending[r] /= None /\ caller[r].st \in {"dropping", "done", "cancelled"}) ~> pending[r] = None

\* R24, R25: a service never stays routed to a connection that can no longer
\* serve it (run() over, or drain task dead).
DeadUnregistered ==
    \A s \in Services, c \in Conns :
        (services[s] = c /\ (Finished(c) \/ ~conn[c].drain)) ~> services[s] /= c

(***************************************************************************)
(* Flapping (spec/REQUIREMENTS.md, "Known consequence"): two live clients   *)
(* for the same service that keep reconnecting. Checked with Flapping.cfg,  *)
(* where Settles is expected to be VIOLATED.                                *)
(***************************************************************************)

GoodAuth(s) == [type |-> "auth", svc |-> s, token |-> "right"]

\* An eager client: connects, authenticates for s, never closes on its own,
\* and reconnects as soon as its connection ends.
EagerClient(c, s) ==
    /\ WF_vars(Connect(c))
    /\ WF_vars(RunStart(c))
    /\ WF_vars(Push(c, GoodAuth(s)) /\ conn[c].pushed = 0)
    /\ WF_vars(Reconnect(c))

\* Clients never close on their own: any replacement is caused by newest-wins.
FlapNext == Next /\ \A c \in Conns : ~Close(c)

FlapSpec ==
    Init /\ [][FlapNext]_vars /\ Fairness
        /\ \A c \in Conns : \A s \in Services : EagerClient(c, s)

\* A connection that is still open is being replaced (between Register and Kick).
LiveReplacement ==
    \E c \in Conns : conn[c].phase = "kick" /\ conn[c].old /= None /\ conn[conn[c].old].open

\* Eventually no live client is ever replaced again.
Settles == <>[](~LiveReplacement)

\* Symmetry for safety-only configs (unsound for liveness in TLC).
Symmetry == Permutations(Conns) \cup Permutations(Reqs) \cup Permutations(Services)

(***************************************************************************)
(* Witnesses: each must be VIOLATED, proving the behavior is reachable      *)
(* (guards against a vacuous spec). Checked with Witness.cfg.               *)
(***************************************************************************)

NeverOk == \A r \in Reqs : caller[r].res /= "ok"
NeverTimeout == \A r \in Reqs : caller[r].res /= "504"
NeverDisconnected502 == \A r \in Reqs : ~(caller[r].res = "502" /\ caller[r].conn /= None)
NeverReplaced == \A c \in Conns : conn[c].old = None
NeverCancelled == \A r \in Reqs : caller[r].st /= "cancelled"
NeverDroppedWhilePending == \A r \in Reqs : ~(caller[r].st = "dropping" /\ pending[r] /= None)
NeverNotConnected == \A r \in Reqs : ~(caller[r].res = "502" /\ caller[r].conn = None)

=============================================================================
