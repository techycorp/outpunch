------------------------------ MODULE RubyGvl ------------------------------
(***************************************************************************)
(* Model of the Ruby native binding's blocking-call protocol:              *)
(*   bindings/ruby/ext/outpunch/src/gvl.rs     without_gvl                 *)
(*   bindings/ruby/ext/outpunch/src/server.rs  wait_for, handle_request,   *)
(*                                             push_message, next_outgoing *)
(*                                                                         *)
(* Each Ruby thread makes one blocking call: push_message ("push"),        *)
(* handle_request ("wait": wait_for on a spawned task), or next_outgoing   *)
(* ("recv": locks the connection's outgoing receiver, then receives).      *)
(* is_connected has the same shape as "push" with an always-ready op and   *)
(* is not modeled separately. run() is not modeled (see spec/README.md).   *)
(*                                                                         *)
(* Ruby C API semantics assumed (Ruby 3.4.8; quotes are verbatim):         *)
(*                                                                         *)
(* A1. rb_thread_call_without_gvl2 skips func when an interrupt is pending *)
(*     at entry, and does not check interrupts after func:                 *)
(*     "(1) Check interrupt and return if interrupted."                    *)
(*     "If 'rb_thread_call_without_gvl2()' detects interrupt, it returns   *)
(*     immediately."                                                       *)
(*     https://raw.githubusercontent.com/ruby/ruby/v3_4_8/thread.c         *)
(*     The entry check is RUBY_VM_INTERRUPTED_ANY, which includes the      *)
(*     timer bit (unblock_function_set, fail_if_interrupted), and          *)
(*     rb_nogvl only runs RUBY_VM_CHECK_INTS_BLOCKING when                 *)
(*     (flags & RB_NOGVL_INTR_FAIL) == 0 (same file). The check and the    *)
(*     ubf registration happen under th->interrupt_lock, the same lock an  *)
(*     interrupter holds, so Enter is one atomic step here.                *)
(*                                                                         *)
(* A2. Interrupts from other threads set a flag and call the registered    *)
(*     ubf: threadptr_interrupt_locked does RUBY_VM_SET_INTERRUPT(th->ec)  *)
(*     then "(th->unblock.func)(th->unblock.arg)" (thread.c). Thread#raise *)
(*     enqueues the exception then calls rb_threadptr_interrupt;           *)
(*     Thread#kill enqueues RUBY_FATAL_THREAD_KILLED then interrupts;      *)
(*     Thread#wakeup (rb_threadptr_ready) only interrupts (thread.c).      *)
(*     The VM timer only ORs a flag:                                       *)
(*     "#define RUBY_VM_SET_TIMER_INTERRUPT(ec) ATOMIC_OR((ec)->interrupt_flag, TIMER_INTERRUPT_MASK)" *)
(*     https://raw.githubusercontent.com/ruby/ruby/v3_4_8/vm_core.h        *)
(*                                                                         *)
(* A3. rb_thread_check_ints clears the unmasked flags                      *)
(*     (threadptr_get_interrupts CASes the flag to "interrupt &            *)
(*     ec->interrupt_mask") and raises a queued exception                  *)
(*     ("rb_exc_raise(err)" in rb_threadptr_execute_interrupts); an        *)
(*     interrupt with an empty queue (Thread#wakeup, timer) raises nothing *)
(*     (thread.c). magnus: "Check for, and run, pending interrupts."       *)
(*     returning Result<(), Error>                                         *)
(*     https://docs.rs/magnus/0.8.2/magnus/struct.Ruby.html                *)
(*     Not modeled: Thread.handle_interrupt masks (exceptions deferred).   *)
(*                                                                         *)
(* A4. tokio (https://docs.rs/tokio/latest/tokio/...):                     *)
(*     Notify stores the ubf's wakeup if func is not waiting yet:          *)
(*       "notify_one() sets the permit, waking a pending task if there is one." *)
(*       (sync/struct.Notify.html)                                         *)
(*     A cancelled send sends nothing: "and another branch completes       *)
(*       first, then it is guaranteed that the message was not sent."      *)
(*       (sync/mpsc/struct.Sender.html)                                    *)
(*     A cancelled recv receives nothing: "and another branch completes    *)
(*       first, it is guaranteed that no messages were received on this    *)
(*       channel." (sync/mpsc/struct.UnboundedReceiver.html)               *)
(*     "Awaiting a &mut JoinHandle<T> is cancel safe."                     *)
(*       (task/struct.JoinHandle.html)                                     *)
(*     Abort drops the task's future: "All local variables are destroyed   *)
(*       by running their destructor." (task/index.html)                   *)
(*     tokio::select! with both branches ready picks either: modeled as a  *)
(*     nondeterministic choice between FuncOp and FuncCancel.              *)
(*                                                                         *)
(* Variant selects the code being modeled:                                 *)
(*   "current"      the binding as it is                                   *)
(*   "pre052skip"   before v0.5.2: a call whose func was skipped is        *)
(*                  treated as completed (CHANGELOG.md v0.5.2)             *)
(*   "pre052wakeup" before v0.5.2: an interrupt that raises nothing ends   *)
(*                  the call (CHANGELOG.md v0.5.2: "Thread#wakeup also     *)
(*                  closed a running connection and failed waiting         *)
(*                  requests")                                             *)
(*   "noguard"      the core without PendingGuard (F1, spec/FINDINGS.md)   *)
(***************************************************************************)
EXTENDS Naturals

CONSTANTS
    T1, T2,         \* the two Ruby threads (model values)
    Kind1, Kind2,   \* "push" | "wait" | "recv": the call each thread makes
    MaxInts,        \* bound on interrupts delivered in a behavior
    MaxMsgs,        \* bound on outgoing messages produced (recv)
    Variant,
    NoThread        \* model value

Threads == {T1, T2}
Kind == [t \in Threads |-> IF t = T1 THEN Kind1 ELSE Kind2]

VARIABLES
    pc,         \* where the thread is in its call
    res,        \* "none" while in the call; "ok" (returned a value), "raised", "ended" (returned with neither)
    out,        \* result of the last native call: "none" | "some" | "cancel" | "skip"
    flag,       \* ec->interrupt_flag, abstracted to SUBSET {"timer", "pending"}
    queued,     \* an exception (Thread#raise / #kill / Timeout) is in the pending-interrupt queue
    ubf,        \* the unblock function is registered (inside the blocking region)
    permit,     \* the Notify permit of the current attempt
    opDone,     \* times the call's operation took effect (message sent, message received, task joined)
    raises,     \* ghost: exceptions raised into the thread
    retried,    \* ghost: the call went round its retry loop
    skipped,    \* ghost: func was skipped at least once
    cancelled,  \* ghost: func was cancelled by the ubf at least once
    ints,       \* interrupts delivered so far
    ready,      \* push: the core's incoming channel has capacity
    task,       \* wait: "none" | "running" | "finished" | "aborted" | "dropped"
    pending,    \* wait: the core's pending-map entry for the request exists
    lockOwner,  \* recv: holder of the connection's outgoing-receiver Mutex
    msgs,       \* recv: messages waiting on the outgoing channel
    produced    \* recv: messages produced so far

vars == <<pc, res, out, flag, queued, ubf, permit, opDone, raises, retried, skipped,
          cancelled, ints, ready, task, pending, lockOwner, msgs, produced>>

TypeOK ==
    /\ pc \in [Threads -> {"enter", "lock", "func", "funcret", "check", "raised", "end"}]
    /\ res \in [Threads -> {"none", "ok", "raised", "ended"}]
    /\ out \in [Threads -> {"none", "some", "cancel", "skip"}]
    /\ flag \in [Threads -> SUBSET {"timer", "pending"}]
    /\ queued \in [Threads -> BOOLEAN]
    /\ ubf \in [Threads -> BOOLEAN]
    /\ permit \in [Threads -> BOOLEAN]
    /\ opDone \in [Threads -> 0..2]
    /\ raises \in [Threads -> 0..MaxInts]
    /\ retried \in [Threads -> BOOLEAN]
    /\ skipped \in [Threads -> BOOLEAN]
    /\ cancelled \in [Threads -> BOOLEAN]
    /\ ints \in 0..MaxInts
    /\ ready \in [Threads -> BOOLEAN]
    /\ task \in [Threads -> {"none", "running", "finished", "aborted", "dropped"}]
    /\ pending \in [Threads -> BOOLEAN]
    /\ lockOwner \in Threads \cup {NoThread}
    /\ msgs \in 0..MaxMsgs
    /\ produced \in 0..MaxMsgs

Init ==
    /\ pc = [t \in Threads |-> "enter"]
    /\ res = [t \in Threads |-> "none"]
    /\ out = [t \in Threads |-> "none"]
    /\ flag = [t \in Threads |-> {}]
    /\ queued = [t \in Threads |-> FALSE]
    /\ ubf = [t \in Threads |-> FALSE]
    /\ permit = [t \in Threads |-> FALSE]
    /\ opDone = [t \in Threads |-> 0]
    /\ raises = [t \in Threads |-> 0]
    /\ retried = [t \in Threads |-> FALSE]
    /\ skipped = [t \in Threads |-> FALSE]
    /\ cancelled = [t \in Threads |-> FALSE]
    /\ ints = 0
    /\ ready = [t \in Threads |-> FALSE]
    /\ task = [t \in Threads |-> IF Kind[t] = "wait" THEN "running" ELSE "none"]
    /\ pending = [t \in Threads |-> Kind[t] = "wait"]
    /\ lockOwner = NoThread
    /\ msgs = 0
    /\ produced = 0

InCall(t) == pc[t] \notin {"raised", "end"}

(***************************************************************************)
(* Binding (Ruby thread t)                                                  *)
(***************************************************************************)

\* One loop iteration of without_gvl: rb_thread_call_without_gvl2 with a fresh
\* Notify. Interrupt pending: func skipped (A1). Otherwise the ubf is
\* registered and func starts; "recv" first awaits the receiver Mutex.
Enter(t) ==
    /\ pc[t] = "enter"
    /\ permit' = [permit EXCEPT ![t] = FALSE]
    /\ IF flag[t] /= {}
       THEN /\ skipped' = [skipped EXCEPT ![t] = TRUE]
            /\ IF Variant = "pre052skip"
               THEN /\ pc' = [pc EXCEPT ![t] = "end"]
                    /\ res' = [res EXCEPT ![t] = "ok"]
                    /\ out' = [out EXCEPT ![t] = "some"]
               ELSE /\ pc' = [pc EXCEPT ![t] = "check"]
                    /\ out' = [out EXCEPT ![t] = "skip"]
                    /\ UNCHANGED res
            /\ UNCHANGED ubf
       ELSE /\ ubf' = [ubf EXCEPT ![t] = TRUE]
            /\ pc' = [pc EXCEPT ![t] = IF Kind[t] = "recv" THEN "lock" ELSE "func"]
            /\ UNCHANGED <<res, out, skipped>>
    /\ UNCHANGED <<flag, queued, opDone, raises, retried, cancelled, ints, ready,
                   task, pending, lockOwner, msgs, produced>>

\* next_outgoing: `outgoing.lock().await` runs inside the select!, next to
\* cancel.notified(), so the Notify can cancel the wait for the lock (F2 fix).
Lock(t) ==
    /\ pc[t] = "lock"
    /\ lockOwner = NoThread
    /\ lockOwner' = t
    /\ pc' = [pc EXCEPT ![t] = "func"]
    /\ UNCHANGED <<res, out, flag, queued, ubf, permit, opDone, raises, retried, skipped,
                   cancelled, ints, ready, task, pending, msgs, produced>>

\* The select! completes on cancel.notified() while still waiting for the lock:
\* the lock() future is dropped without acquiring the lock.
LockCancel(t) ==
    /\ pc[t] = "lock"
    /\ permit[t]
    /\ permit' = [permit EXCEPT ![t] = FALSE]
    /\ cancelled' = [cancelled EXCEPT ![t] = TRUE]
    /\ out' = [out EXCEPT ![t] = "cancel"]
    /\ pc' = [pc EXCEPT ![t] = "funcret"]
    /\ UNCHANGED <<res, flag, queued, ubf, opDone, raises, retried, skipped, ints, ready,
                   task, pending, lockOwner, msgs, produced>>

OpReady(t) ==
    CASE Kind[t] = "push" -> ready[t]
      [] Kind[t] = "wait" -> task[t] = "finished"
      [] Kind[t] = "recv" -> msgs > 0

\* The select! completes on the operation: the message is sent, a message is
\* received, or the task is joined.
FuncOp(t) ==
    /\ pc[t] = "func"
    /\ OpReady(t)
    /\ opDone' = [opDone EXCEPT ![t] = @ + 1]
    /\ ready' = IF Kind[t] = "push" THEN [ready EXCEPT ![t] = FALSE] ELSE ready
    /\ msgs' = IF Kind[t] = "recv" THEN msgs - 1 ELSE msgs
    /\ out' = [out EXCEPT ![t] = "some"]
    /\ pc' = [pc EXCEPT ![t] = "funcret"]
    /\ UNCHANGED <<res, flag, queued, ubf, permit, raises, retried, skipped, cancelled,
                   ints, task, pending, lockOwner, produced>>

\* The select! completes on cancel.notified(): nothing was sent or received (A4).
FuncCancel(t) ==
    /\ pc[t] = "func"
    /\ permit[t]
    /\ permit' = [permit EXCEPT ![t] = FALSE]
    /\ cancelled' = [cancelled EXCEPT ![t] = TRUE]
    /\ out' = [out EXCEPT ![t] = "cancel"]
    /\ pc' = [pc EXCEPT ![t] = "funcret"]
    /\ UNCHANGED <<res, flag, queued, ubf, opDone, raises, retried, skipped, ints, ready,
                   task, pending, lockOwner, msgs, produced>>

\* func returns (dropping the Mutex guard); the blocking region ends. A value
\* is returned to Ruby without checking interrupts; a cancel goes to check.
RegionEnd(t) ==
    /\ pc[t] = "funcret"
    /\ ubf' = [ubf EXCEPT ![t] = FALSE]
    /\ lockOwner' = IF lockOwner = t THEN NoThread ELSE lockOwner
    /\ IF out[t] = "some"
       THEN /\ pc' = [pc EXCEPT ![t] = "end"]
            /\ res' = [res EXCEPT ![t] = "ok"]
       ELSE /\ pc' = [pc EXCEPT ![t] = "check"]
            /\ UNCHANGED res
    /\ UNCHANGED <<out, flag, queued, permit, opDone, raises, retried, skipped, cancelled,
                   ints, ready, task, pending, msgs, produced>>

\* ruby.thread_check_ints() (A3): raise a queued exception, otherwise loop.
Check(t) ==
    /\ pc[t] = "check"
    /\ flag' = [flag EXCEPT ![t] = {}]
    /\ IF queued[t]
       THEN /\ queued' = [queued EXCEPT ![t] = FALSE]
            /\ pc' = [pc EXCEPT ![t] = "raised"]
            /\ res' = [res EXCEPT ![t] = "raised"]
            /\ UNCHANGED retried
       ELSE /\ UNCHANGED queued
            /\ IF Variant = "pre052wakeup" /\ out[t] = "cancel"
               THEN /\ pc' = [pc EXCEPT ![t] = "end"]
                    /\ res' = [res EXCEPT ![t] = "ended"]
                    /\ UNCHANGED retried
               ELSE /\ pc' = [pc EXCEPT ![t] = "enter"]
                    /\ retried' = [retried EXCEPT ![t] = TRUE]
                    /\ UNCHANGED res
    /\ UNCHANGED <<out, ubf, permit, opDone, raises, skipped, cancelled, ints, ready,
                   task, pending, lockOwner, msgs, produced>>

\* The exception propagates. handle_request aborts its task first.
Unwind(t) ==
    /\ pc[t] = "raised"
    /\ task' = IF task[t] = "running" THEN [task EXCEPT ![t] = "aborted"] ELSE task
    /\ pc' = [pc EXCEPT ![t] = "end"]
    /\ UNCHANGED <<res, out, flag, queued, ubf, permit, opDone, raises, retried, skipped,
                   cancelled, ints, ready, pending, lockOwner, msgs, produced>>

(***************************************************************************)
(* Tokio runtime and core                                                   *)
(***************************************************************************)

\* handle_request's task finishes: the core returns a response or a tunnel
\* error, removing its pending entry (Tunnel.tla: EveryRequestEnds).
TaskFinish(t) ==
    /\ task[t] = "running"
    /\ task' = [task EXCEPT ![t] = "finished"]
    /\ pending' = [pending EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<pc, res, out, flag, queued, ubf, permit, opDone, raises, retried,
                   skipped, cancelled, ints, ready, lockOwner, msgs, produced>>

\* An aborted task's future is dropped; PendingGuard removes the entry (A4).
TaskDrop(t) ==
    /\ task[t] = "aborted"
    /\ task' = [task EXCEPT ![t] = "dropped"]
    /\ pending' = IF Variant = "noguard" THEN pending ELSE [pending EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<pc, res, out, flag, queued, ubf, permit, opDone, raises, retried,
                   skipped, cancelled, ints, ready, lockOwner, msgs, produced>>

\* The core's incoming channel gets capacity for a blocked push.
ChannelReady(t) ==
    /\ Kind[t] = "push"
    /\ opDone[t] = 0
    /\ ~ready[t]
    /\ ready' = [ready EXCEPT ![t] = TRUE]
    /\ UNCHANGED <<pc, res, out, flag, queued, ubf, permit, opDone, raises, retried,
                   skipped, cancelled, ints, task, pending, lockOwner, msgs, produced>>

(***************************************************************************)
(* Environment: other Ruby threads, the VM timer, the tunnel client          *)
(***************************************************************************)

\* Interrupt t; calls the ubf if registered (A2).
Interrupt(t, f, exc) ==
    /\ InCall(t)
    /\ ints < MaxInts
    /\ ints' = ints + 1
    /\ flag' = [flag EXCEPT ![t] = @ \cup {f}]
    /\ queued' = IF exc THEN [queued EXCEPT ![t] = TRUE] ELSE queued
    /\ raises' = IF exc THEN [raises EXCEPT ![t] = @ + 1] ELSE raises
    /\ permit' = IF ubf[t] /\ f = "pending" THEN [permit EXCEPT ![t] = TRUE] ELSE permit
    /\ UNCHANGED <<pc, res, out, ubf, opDone, retried, skipped, cancelled, ready, task,
                   pending, lockOwner, msgs, produced>>

Timer(t) == Interrupt(t, "timer", FALSE)       \* GVL contention
Wakeup(t) == Interrupt(t, "pending", FALSE)    \* Thread#wakeup
Raise(t) == Interrupt(t, "pending", TRUE)      \* Thread#raise, Thread#kill, Timeout

\* The core queues a message for the tunnel client (no fairness: an idle
\* connection may send nothing for a long time).
MsgArrive ==
    /\ produced < MaxMsgs
    /\ msgs' = msgs + 1
    /\ produced' = produced + 1
    /\ UNCHANGED <<pc, res, out, flag, queued, ubf, permit, opDone, raises, retried,
                   skipped, cancelled, ints, ready, task, pending, lockOwner>>

(***************************************************************************)
(* Specification                                                           *)
(***************************************************************************)

Binding(t) ==
    Enter(t) \/ Lock(t) \/ LockCancel(t) \/ FuncOp(t) \/ FuncCancel(t) \/ RegionEnd(t) \/ Check(t) \/ Unwind(t)

Runtime(t) == TaskFinish(t) \/ TaskDrop(t) \/ ChannelReady(t)

Next ==
    \/ \E t \in Threads : Binding(t) \/ Runtime(t) \/ Timer(t) \/ Wakeup(t) \/ Raise(t)
    \/ MsgArrive

\* Binding and runtime steps are fair; interrupts and messages are not.
Fairness ==
    \A t \in Threads :
        /\ WF_vars(Enter(t)) /\ WF_vars(Lock(t)) /\ WF_vars(LockCancel(t)) /\ WF_vars(FuncOp(t))
        /\ WF_vars(FuncCancel(t)) /\ WF_vars(RegionEnd(t)) /\ WF_vars(Check(t))
        /\ WF_vars(Unwind(t))
        /\ WF_vars(TaskFinish(t)) /\ WF_vars(TaskDrop(t)) /\ WF_vars(ChannelReady(t))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* Properties                                                              *)
(***************************************************************************)

\* A call returns a value only if its operation took effect (v0.5.2: a
\* skipped call was treated as completed).
ResultOnlyIfRan == \A t \in Threads : res[t] = "ok" => opDone[t] >= 1

\* A retried call never repeats its operation (no duplicate sends/receives).
NoDuplicateOp == \A t \in Threads : opDone[t] <= 1

\* Only an exception raised into the thread makes the call raise.
RaisedOnlyByException == \A t \in Threads : res[t] = "raised" => raises[t] >= 1

\* An interrupt that raises nothing (Thread#wakeup, the VM timer) never ends
\* the call: it returns a value or raises, nothing else.
NoEndWithoutResult == \A t \in Threads : res[t] /= "ended"

Safety ==
    /\ ResultOnlyIfRan /\ NoDuplicateOp /\ RaisedOnlyByException /\ NoEndWithoutResult

\* Every call returns, given its operation eventually becomes ready (push,
\* wait; recv waits for a message by contract and is excluded).
EveryCallEnds ==
    \A t \in Threads : Kind[t] \in {"push", "wait"} => <>(res[t] /= "none")

\* An exception raised into a thread ends its call, whatever the operation.
ExceptionStopsCall == \A t \in Threads : (queued[t] /\ res[t] = "none") ~> res[t] /= "none"

\* A request whose handle_request raised leaves no pending entry in the core.
NoLeak == \A t \in Threads : task[t] = "aborted" ~> ~pending[t]

(***************************************************************************)
(* Witnesses: each must be VIOLATED (reachability)                          *)
(***************************************************************************)

NeverOk == \A t \in Threads : res[t] /= "ok"
NeverRaised == \A t \in Threads : res[t] /= "raised"
NeverSkipped == \A t \in Threads : ~skipped[t]
NeverCancelled == \A t \in Threads : ~cancelled[t]
NeverRetriedThenOk == \A t \in Threads : ~(retried[t] /\ res[t] = "ok")
NeverAborted == \A t \in Threads : task[t] \notin {"aborted", "dropped"}

=============================================================================
