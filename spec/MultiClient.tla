----------------------------- MODULE MultiClient -----------------------------
(***************************************************************************)
(* Design model (not built): multiple clients per service, keyed by client  *)
(* instance ID. See spec/notes/multi-client.md. Derived from spec/Tunnel.tla *)
(* (same request lifecycle, including the PendingGuard fix for F1) so the   *)
(* two stay comparable. Differences from Tunnel.tla:                        *)
(*                                                                         *)
(*   - services maps each service to a SET of registered connections.      *)
(*   - auth carries an instance ID, or none. The ID is adversarial: any    *)
(*     connection may present any ID from Ids, or None.                    *)
(*   - Register replaces only connections of the same service with the    *)
(*     same non-None ID; all others coexist.                               *)
(*   - Start routes to ANY registered connection (covers round-robin,      *)
(*     failover, and any other selection policy).                          *)
(*   - A socket can die silently (Die): the client can no longer push, and *)
(*     requests sent to it are lost, but the server has not noticed.       *)
(*   - Evict: the server's ping timeout notices a dead socket and closes   *)
(*     it. Weakly fair. Assumption: the detector closes only dead sockets; *)
(*     spurious closes of live sockets are covered by the environment's    *)
(*     Close action.                                                       *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets, TLC

CONSTANTS
    Conns,      \* connection ids
    Services,   \* service names
    Reqs,       \* request ids
    Ids,        \* client instance IDs a connection may present
    Tokens,     \* auth tokens a client may send; "right" is the server's secret
    SecretSet,  \* TRUE: server has a non-empty secret
    MaxMsgs,    \* bound on messages each connection may push
    None        \* model value

\* Malformed messages and wrong tokens are covered by Tunnel.tla; the multi-client
\* configs set Tokens = {"right"} and omit malformed messages to keep the state
\* space small.
Msgs ==
    [type : {"auth"}, svc : Services, token : Tokens, iid : Ids \cup {None}]
        \cup [type : {"resp"}, req : Reqs]

Phases == {"idle", "auth", "reg", "kick", "relay", "cleanup", "done"}
CallerStates == {"idle", "sending", "waiting", "removing", "dropping", "done", "cancelled"}

VARIABLES
    services,   \* Services -> SUBSET Conns: registered connections
    pending,    \* Reqs -> Conns \cup {None}
    conn,       \* Conns -> per-connection state
    caller,     \* Reqs -> state of the dispatch future
    oneshot,    \* Reqs -> state of the response oneshot
    delivered,  \* request ids that reached some live client
    authed,     \* ghost: Conns -> BOOLEAN
    deadPick    \* ghost: Reqs -> BOOLEAN, routed to a dead socket while a live one was registered

vars == <<services, pending, conn, caller, oneshot, delivered, authed, deadPick>>

TypeOK ==
    /\ services \in [Services -> SUBSET Conns]
    /\ pending \in [Reqs -> Conns \cup {None}]
    /\ \A c \in Conns :
        /\ conn[c].phase \in Phases
        /\ conn[c].open \in BOOLEAN
        /\ conn[c].alive \in BOOLEAN
        /\ conn[c].evicted \in BOOLEAN
        /\ Len(conn[c].inbox) <= MaxMsgs
        /\ \A i \in 1..Len(conn[c].inbox) : conn[c].inbox[i] \in Msgs
        /\ conn[c].svc \in Services \cup {None}
        /\ conn[c].iid \in Ids \cup {None}
        /\ conn[c].old \subseteq Conns
        /\ conn[c].pushed \in 0..MaxMsgs
    /\ \A r \in Reqs :
        /\ caller[r].st \in CallerStates
        /\ caller[r].conn \in Conns \cup {None}
        /\ caller[r].res \in {"none", "ok", "502", "504"}
        /\ caller[r].from \in Conns \cup {None}
    /\ \A r \in Reqs :
        /\ oneshot[r].st \in {"empty", "value", "closed"}
        /\ oneshot[r].from \in Conns \cup {None}
    /\ delivered \subseteq Reqs
    /\ authed \in [Conns -> BOOLEAN]
    /\ deadPick \in [Reqs -> BOOLEAN]

Init ==
    /\ services = [s \in Services |-> {}]
    /\ pending = [r \in Reqs |-> None]
    /\ conn = [c \in Conns |->
                [phase |-> "idle", open |-> FALSE, alive |-> FALSE, evicted |-> FALSE,
                 inbox |-> <<>>, svc |-> None, iid |-> None, old |-> {}, pushed |-> 0]]
    /\ caller = [r \in Reqs |-> [st |-> "idle", conn |-> None, res |-> "none", from |-> None]]
    /\ oneshot = [r \in Reqs |-> [st |-> "empty", from |-> None]]
    /\ delivered = {}
    /\ authed = [c \in Conns |-> FALSE]
    /\ deadPick = [r \in Reqs |-> FALSE]

DropSender(os, rs) ==
    [r \in Reqs |-> IF r \in rs /\ os[r].st = "empty"
                        THEN [st |-> "closed", from |-> None]
                        ELSE os[r]]

(***************************************************************************)
(* Environment: tunnel clients, sockets, and the adapter                   *)
(***************************************************************************)

Connect(c) ==
    /\ conn[c].phase = "idle"
    /\ conn' = [conn EXCEPT ![c].phase = "auth", ![c].open = TRUE, ![c].alive = TRUE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* Only a live socket carries messages from the client.
Push(c, m) ==
    /\ conn[c].open
    /\ conn[c].alive
    /\ conn[c].phase \notin {"idle", "done"}
    /\ conn[c].pushed < MaxMsgs
    /\ m.type = "resp" => m.req \in delivered
    /\ conn' = [conn EXCEPT ![c].inbox = Append(@, m), ![c].pushed = @ + 1]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* The adapter notices the socket closed (or a spurious close).
Close(c) ==
    /\ conn[c].open
    /\ conn' = [conn EXCEPT ![c].open = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* The socket dies silently: nothing more flows, and the server has not noticed.
Die(c) ==
    /\ conn[c].alive
    /\ conn' = [conn EXCEPT ![c].alive = FALSE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

(***************************************************************************)
(* Server: connection lifecycle                                            *)
(***************************************************************************)

Valid(m) == m.type = "auth" /\ SecretSet /\ m.token = "right"

Auth(c) ==
    /\ conn[c].phase = "auth"
    /\ conn[c].inbox /= <<>>
    /\ LET m == Head(conn[c].inbox) IN
        IF Valid(m)
        THEN /\ conn' = [conn EXCEPT ![c].inbox = Tail(@), ![c].phase = "reg",
                                     ![c].svc = m.svc, ![c].iid = m.iid]
             /\ authed' = [authed EXCEPT ![c] = TRUE]
        ELSE /\ conn' = [conn EXCEPT ![c].inbox = Tail(@), ![c].phase = "done"]
             /\ UNCHANGED authed
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, deadPick>>

AuthTimeout(c) ==
    /\ conn[c].phase = "auth"
    /\ conn[c].inbox = <<>>
    /\ conn' = [conn EXCEPT ![c].phase = "done"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* Same service and same non-None instance ID: replace. Everything else coexists.
SameInstance(o, c) ==
    /\ conn[c].iid /= None
    /\ conn[o].iid = conn[c].iid

Register(c) ==
    /\ conn[c].phase = "reg"
    /\ LET s == conn[c].svc
           olds == {o \in services[s] : SameInstance(o, c)}
       IN
        /\ services' = [services EXCEPT ![s] = (@ \ olds) \cup {c}]
        /\ conn' = [conn EXCEPT ![c].phase = "kick", ![c].old = olds]
    /\ UNCHANGED <<pending, caller, oneshot, delivered, authed, deadPick>>

\* Outside the lock: close every replaced connection.
Kick(c) ==
    /\ conn[c].phase = "kick"
    /\ conn' = [o \in Conns |->
                  IF o = c THEN [conn[c] EXCEPT !.phase = "relay"]
                  ELSE IF o \in conn[c].old THEN [conn[o] EXCEPT !.open = FALSE]
                  ELSE conn[o]]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* Ping timeout: the server notices a dead socket and closes it.
Evict(c) ==
    /\ ~conn[c].alive
    /\ conn[c].open
    /\ conn[c].phase \in {"reg", "kick", "relay"}
    /\ conn' = [conn EXCEPT ![c].open = FALSE, ![c].evicted = TRUE]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

Relay(c) ==
    /\ conn[c].phase = "relay"
    /\ conn[c].inbox /= <<>>
    /\ LET m == Head(conn[c].inbox) IN
        IF m.type = "resp" /\ pending[m.req] = c
        THEN /\ pending' = [pending EXCEPT ![m.req] = None]
             /\ oneshot' = [oneshot EXCEPT ![m.req] = [st |-> "value", from |-> c]]
        ELSE UNCHANGED <<pending, oneshot>>
    /\ conn' = [conn EXCEPT ![c].inbox = Tail(@)]
    /\ UNCHANGED <<services, caller, delivered, authed, deadPick>>

RelayEnd(c) ==
    /\ conn[c].phase = "relay"
    /\ conn[c].inbox = <<>>
    /\ ~conn[c].open
    /\ conn' = [conn EXCEPT ![c].phase = "cleanup"]
    /\ UNCHANGED <<services, pending, caller, oneshot, delivered, authed, deadPick>>

\* Unregister only itself; fail its pending requests.
Cleanup(c) ==
    /\ conn[c].phase = "cleanup"
    /\ LET s == conn[c].svc
           mine == {r \in Reqs : pending[r] = c}
       IN
        /\ services' = [services EXCEPT ![s] = @ \ {c}]
        /\ pending' = [r \in Reqs |-> IF r \in mine THEN None ELSE pending[r]]
        /\ oneshot' = DropSender(oneshot, mine)
    /\ conn' = [conn EXCEPT ![c].phase = "done"]
    /\ UNCHANGED <<caller, delivered, authed, deadPick>>

(***************************************************************************)
(* Server: dispatch for request r                                          *)
(***************************************************************************)

Live(c) == conn[c].alive /\ conn[c].open

\* Route to any registered connection; 502 only when none is registered.
Start(r, s) ==
    /\ caller[r].st = "idle"
    /\ IF services[s] = {}
       THEN /\ caller' = [caller EXCEPT ![r].st = "done", ![r].res = "502"]
            /\ UNCHANGED <<pending, deadPick>>
       ELSE \E c \in services[s] :
            /\ pending' = [pending EXCEPT ![r] = c]
            /\ caller' = [caller EXCEPT ![r].st = "sending", ![r].conn = c]
            /\ deadPick' = [deadPick EXCEPT ![r] =
                              ~conn[c].alive /\ \E o \in services[s] : conn[o].alive]
    /\ UNCHANGED <<services, conn, oneshot, delivered, authed>>

\* The send completes. A dead socket swallows the request.
Send(r) ==
    /\ caller[r].st = "sending"
    /\ caller' = [caller EXCEPT ![r].st = "waiting"]
    /\ delivered' = IF conn[caller[r].conn].alive THEN delivered \cup {r} ELSE delivered
    /\ UNCHANGED <<services, pending, conn, oneshot, authed, deadPick>>

Receive(r) ==
    /\ caller[r].st = "waiting"
    /\ oneshot[r].st /= "empty"
    /\ caller' = IF oneshot[r].st = "value"
                 THEN [caller EXCEPT ![r].st = "done", ![r].res = "ok", ![r].from = oneshot[r].from]
                 ELSE [caller EXCEPT ![r].st = "removing", ![r].res = "502"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, deadPick>>

Timeout(r) ==
    /\ caller[r].st \in {"sending", "waiting"}
    /\ caller' = [caller EXCEPT ![r].st = "removing", ![r].res = "504"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, deadPick>>

\* PendingGuard removes the entry when dispatch returns.
Remove(r) ==
    /\ caller[r].st = "removing"
    /\ pending' = [pending EXCEPT ![r] = None]
    /\ oneshot' = DropSender(oneshot, {r})
    /\ caller' = [caller EXCEPT ![r].st = "done"]
    /\ UNCHANGED <<services, conn, delivered, authed, deadPick>>

\* The dispatch future is dropped while it awaits; PendingGuard's destructor then takes the
\* lock (other steps may run first) and removes the entry, as in Tunnel.tla.
Cancel(r) ==
    /\ caller[r].st \in {"sending", "waiting"}
    /\ caller' = [caller EXCEPT ![r].st = "dropping"]
    /\ UNCHANGED <<services, pending, conn, oneshot, delivered, authed, deadPick>>

GuardDrop(r) ==
    /\ caller[r].st = "dropping"
    /\ pending' = [pending EXCEPT ![r] = None]
    /\ oneshot' = DropSender(oneshot, {r})
    /\ caller' = [caller EXCEPT ![r].st = "cancelled"]
    /\ UNCHANGED <<services, conn, delivered, authed, deadPick>>

(***************************************************************************)
(* Specification                                                           *)
(***************************************************************************)

Environment ==
    \/ \E c \in Conns : Connect(c) \/ Close(c) \/ Die(c)
    \/ \E c \in Conns, m \in Msgs : Push(c, m)
    \/ \E r \in Reqs, s \in Services : Start(r, s)
    \/ \E r \in Reqs : Send(r) \/ Cancel(r)

Server ==
    \/ \E c \in Conns :
        Auth(c) \/ AuthTimeout(c) \/ Register(c) \/ Kick(c) \/ Evict(c)
            \/ Relay(c) \/ RelayEnd(c) \/ Cleanup(c)
    \/ \E r \in Reqs : Receive(r) \/ Timeout(r) \/ Remove(r) \/ GuardDrop(r)

Next == Environment \/ Server

Fairness ==
    /\ \A c \in Conns :
        /\ WF_vars(Auth(c)) /\ WF_vars(AuthTimeout(c)) /\ WF_vars(Register(c))
        /\ WF_vars(Kick(c)) /\ WF_vars(Evict(c)) /\ WF_vars(Relay(c))
        /\ WF_vars(RelayEnd(c)) /\ WF_vars(Cleanup(c))
    /\ \A r \in Reqs :
        WF_vars(Receive(r)) /\ WF_vars(Timeout(r)) /\ WF_vars(Remove(r)) /\ WF_vars(GuardDrop(r))

Spec == Init /\ [][Next]_vars /\ Fairness

Symmetry == Permutations(Conns) \cup Permutations(Reqs) \cup Permutations(Ids)

(***************************************************************************)
(* Safety: carried over from Tunnel.tla                                    *)
(***************************************************************************)

RoutedOnlyIfAuthed ==
    \A s \in Services, c \in Conns :
        c \in services[s] => authed[c] /\ conn[c].svc = s

EmptySecretRejectsAll == ~SecretSet => \A c \in Conns : ~authed[c]

\* Replaces NoStaleRemoval: only finished connections leave no trace, and a
\* registered connection is never finished.
NoFinishedRegistered ==
    \A s \in Services : \A c \in services[s] : conn[c].phase /= "done"

\* No wrong answer, under any instance-ID behavior.
ResponseFromTarget ==
    \A r \in Reqs : caller[r].res = "ok" => caller[r].from = caller[r].conn

PendingOwnedByTarget ==
    \A r \in Reqs : pending[r] /= None => pending[r] = caller[r].conn

NoPendingOnFinishedConn ==
    \A r \in Reqs : pending[r] /= None => conn[pending[r]].phase /= "done"

NoPendingAfterCancel ==
    \A r \in Reqs : caller[r].st = "cancelled" => pending[r] = None

(***************************************************************************)
(* Safety: design properties                                               *)
(***************************************************************************)

\* Replacement only between connections presenting the same non-None ID:
\* two clients with distinct IDs (or no ID) never displace each other.
ReplaceOnlySameInstance ==
    \A c \in Conns : \A o \in conn[c].old :
        /\ conn[c].iid /= None
        /\ conn[o].iid = conn[c].iid
        /\ conn[o].svc = conn[c].svc

\* An authenticated, open connection is registered, unless it is in the
\* middle of being replaced. With NotConnected decided only when the set is
\* empty, this means: a request gets NotConnected only when no open,
\* authenticated connection exists for the service (other than one being
\* closed by a same-ID replacement, or one not yet registered).
OpenImpliesRegistered ==
    \A c \in Conns :
        (conn[c].phase \in {"kick", "relay"} /\ conn[c].open)
            => \/ c \in services[conn[c].svc]
               \/ \E k \in Conns : conn[k].phase = "kick" /\ c \in conn[k].old

Safety ==
    /\ RoutedOnlyIfAuthed
    /\ EmptySecretRejectsAll
    /\ NoFinishedRegistered
    /\ ResponseFromTarget
    /\ PendingOwnedByTarget
    /\ NoPendingOnFinishedConn
    /\ NoPendingAfterCancel
    /\ ReplaceOnlySameInstance
    /\ OpenImpliesRegistered

(***************************************************************************)
(* Liveness                                                                *)
(***************************************************************************)

InFlight(r) == caller[r].st \in {"sending", "waiting", "removing", "dropping"}

EveryRequestEnds == \A r \in Reqs : InFlight(r) ~> caller[r].st \in {"done", "cancelled"}

NoLeak ==
    \A r \in Reqs :
        (pending[r] /= None /\ caller[r].st \in {"dropping", "done", "cancelled"}) ~> pending[r] = None

\* A dead socket is eventually closed and its connection finishes.
DeadEvicted ==
    \A c \in Conns :
        (~conn[c].alive /\ conn[c].phase \in {"auth", "reg", "kick", "relay", "cleanup"})
            ~> conn[c].phase = "done"

\* A dead socket is eventually unregistered, so it stops attracting requests.
DeadUnregistered ==
    \A s \in Services, c \in Conns :
        (c \in services[s] /\ ~conn[c].alive) ~> c \notin services[s]

(***************************************************************************)
(* Witnesses: each must be VIOLATED (reachability; guards against vacuity) *)
(***************************************************************************)

NeverOk == \A r \in Reqs : caller[r].res /= "ok"
NeverCoexist == \A s \in Services : Cardinality(services[s]) <= 1
NeverReplaced == \A c \in Conns : conn[c].old = {}
\* Duplicated ID on two live clients: a live connection gets replaced (flapping).
NeverLiveReplaced == \A c \in Conns : \A o \in conn[c].old : ~conn[o].alive
NeverEvicted == \A c \in Conns : ~conn[c].evicted
\* The cost of silent death: a request routed to a dead socket while a live one was registered.
NeverDeadPick == \A r \in Reqs : ~deadPick[r]
\* Same, for a client that sent no instance ID (an older client). Under today's
\* newest-wins design its reconnect would have replaced the dead socket at once.
NeverDeadPickNoId ==
    \A r \in Reqs : ~(deadPick[r] /\ conn[caller[r].conn].iid = None)
\* A request routed to a dead socket ends with an error, not a hang.
NeverDeadPickFailed == \A r \in Reqs : ~(deadPick[r] /\ caller[r].st = "done" /\ caller[r].res \in {"502", "504"})

=============================================================================
