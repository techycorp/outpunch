---------------------------- MODULE TunnelTrace ----------------------------
(***************************************************************************)
(* Trace validation for Tunnel.tla: is a recorded execution of the Rust     *)
(* core a behavior of the model?                                            *)
(*                                                                         *)
(* The core, built with the `trace` feature, writes one event per model     *)
(* step (crates/outpunch/src/trace.rs). spec/trace/check_traces.py turns a  *)
(* trace into TraceLog and checks with TLC that some behavior of Tunnel.tla *)
(* consumes the whole log: each event must be the next model step, with the *)
(* same parameters and the same observable outcome.                         *)
(*                                                                         *)
(* Three model steps have no event, so they may happen silently: Cancel (a *)
(* dropped dispatch future is not observable until its PendingGuard runs), *)
(* RunDropped (a dropped run() future, observable through the cleanup its  *)
(* guard then runs) and GoSilent (a client that stops sending, observable  *)
(* only through the idle eviction that follows). A dead drain task is      *)
(* traced as drain_death, and the close its guard then performs as close.  *)
(***************************************************************************)
EXTENDS Tunnel

CONSTANTS
    TraceLog,   \* the recorded events, a sequence of records
    Target      \* check that the first Target events can be matched

VARIABLE l      \* index of the next event to match

tvars == <<vars, l>>

E == TraceLog[l]
Ev(kind) == l <= Len(TraceLog) /\ TraceLog[l].ev = kind
Advance == l' = l + 1

TCreate == Ev("create") /\ Connect(E.conn) /\ Advance
TRun == Ev("run") /\ RunStart(E.conn) /\ Advance
TPush == Ev("push") /\ Push(E.conn, E.msg) /\ Advance
TClose == Ev("close") /\ (Close(E.conn) \/ DrainClose(E.conn)) /\ Advance
TAuth == Ev("auth") /\ Auth(E.conn) /\ ((conn'[E.conn].phase = "reg") = E.ok) /\ Advance
TAuthEnd == Ev("auth_end") /\ AuthTimeout(E.conn) /\ Advance
\* Events also carry their effect, checked against the model's state.
TRegister == Ev("register") /\ Register(E.conn) /\ conn'[E.conn].old = E.old /\ Advance
TKick == Ev("kick") /\ Kick(E.conn) /\ Advance
TRelay ==
    /\ Ev("relay") /\ Relay(E.conn)
    /\ IF E.req = None THEN pending' = pending
       ELSE pending[E.req] = E.conn /\ pending'[E.req] = None
    /\ Advance
TRelayEnd == Ev("relay_end") /\ RelayEnd(E.conn) /\ Advance
TCleanup ==
    /\ Ev("cleanup")
    /\ (services[conn[E.conn].svc] = E.conn) = E.unregistered
    /\ {r \in Reqs : pending[r] = E.conn} = E.failed
    /\ Cleanup(E.conn)
    /\ Advance
TStart == Ev("start") /\ Start(E.req, E.svc) /\ caller'[E.req].conn = E.conn /\ Advance
TSend == Ev("send") /\ Send(E.req) /\ Advance
TSendFail == Ev("send_fail") /\ SendFail(E.req) /\ Advance
TReceive == Ev("receive") /\ Receive(E.req) /\ ((caller'[E.req].res = "ok") = E.ok) /\ Advance
TTimeout == Ev("timeout") /\ Timeout(E.req) /\ Advance
TRemove == Ev("remove") /\ Remove(E.req) /\ Advance
TGuardDrop == Ev("guard_drop") /\ GuardDrop(E.req) /\ Advance
TCancel == \E r \in Reqs : Cancel(r) /\ UNCHANGED l
TRunDropped == \E c \in Conns : RunDropped(c) /\ UNCHANGED l
TDrainDeath == Ev("drain_death") /\ DrainDeath(E.conn) /\ Advance
TGoSilent == \E c \in Conns : GoSilent(c) /\ UNCHANGED l
TIdle == Ev("idle") /\ IdleEvict(E.conn) /\ Advance

TInit == Init /\ l = 1

TNext ==
    \/ TCreate \/ TRun \/ TPush \/ TClose \/ TAuth \/ TAuthEnd \/ TRegister \/ TKick
    \/ TRelay \/ TRelayEnd \/ TCleanup \/ TStart \/ TSend \/ TSendFail \/ TReceive
    \/ TTimeout \/ TRemove \/ TGuardDrop \/ TCancel \/ TRunDropped \/ TDrainDeath
    \/ TGoSilent \/ TIdle

\* Violated exactly when the first Target events have been matched.
NotReached == l <= Target

\* The model's safety properties must also hold along the matched trace.
TraceSafety == Safety
=============================================================================
