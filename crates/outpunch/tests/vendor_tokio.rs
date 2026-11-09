//! Facts about tokio that the TLA+ models of the core (spec/Tunnel.tla, spec/AxumBridge.tla)
//! and the PendingGuard fix rely on. Each test observes real tokio; none tests our code.
//! Listed in vendor-claims.toml.
//!
//! Run with: cargo test -p outpunch --test vendor_tokio -- --ignored

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

/// Tunnel.tla relies on this: a resolved response always beats the request timer.
// Claim:    when the inner future and the deadline become ready at the same instant, timeout returns the inner future's output
// Fails if: timeout checked its deadline first; at the shared instant it would return Err(Elapsed)
#[tokio::test(start_paused = true)]
#[ignore = "vendor test"]
async fn tokio_timeout_must_prefer_a_ready_inner_future_over_an_elapsed_deadline() {
    let inner = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "inner"
    };
    let result = tokio::time::timeout(Duration::from_millis(10), inner).await;
    assert_eq!(result, Ok("inner"));
}

/// Tunnel.tla and AxumBridge.tla rely on this: a timer firing during the send means nothing was sent.
// Claim:    an mpsc Sender::send cancelled while waiting for capacity does not deliver its message
// Fails if: a cancelled send still enqueued its message; the second recv would return Some("cancelled")
// Control:  the message sent before the cancellation is received first
#[tokio::test(start_paused = true)]
#[ignore = "vendor test"]
async fn tokio_mpsc_send_must_not_deliver_when_cancelled_while_waiting_for_capacity() {
    let (tx, mut rx) = mpsc::channel::<&str>(1);
    tx.send("fill").await.unwrap();

    let cancelled = tokio::time::timeout(Duration::from_millis(10), tx.send("cancelled")).await;
    assert!(
        cancelled.is_err(),
        "the send should still be waiting when the timer fires"
    );

    assert_eq!(rx.recv().await, Some("fill"));
    drop(tx);
    assert_eq!(rx.recv().await, None, "the cancelled message was delivered");
}

/// Tunnel.tla relies on this: cleanup_connection dropping a pending entry fails the waiting request (502).
// Claim:    dropping a oneshot Sender makes its receiver resolve to an error
// Fails if: the receiver kept waiting after the sender was dropped; the await would never return
#[tokio::test]
#[ignore = "vendor test"]
async fn tokio_oneshot_receiver_must_fail_when_its_sender_is_dropped() {
    let (tx, rx) = oneshot::channel::<()>();
    drop(tx);
    assert!(rx.await.is_err());
}

/// PendingGuard (F1 fix) relies on this when a binding aborts a request task.
// Claim:    aborting a task runs the destructors of the values its future holds
// Fails if: abort leaked the future without dropping it; the flag would stay false
// Control:  the task is first observed running, and the join error reports cancellation
#[tokio::test]
#[ignore = "vendor test"]
async fn tokio_abort_must_run_destructors_of_the_tasks_locals() {
    let dropped = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = oneshot::channel();
    let task = tokio::spawn({
        let guard = SetOnDrop(dropped.clone());
        async move {
            let _guard = guard;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        }
    });
    started_rx.await.unwrap();

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
}

/// RubyGvl.tla relies on this: the binding's unblocking function calls notify_one before the
/// waiting select! may be listening; the permit must survive until it is (finding F2).
// Claim:    notify_one with no task waiting stores a permit that the next notified() consumes at once
// Fails if: notify_one with no waiter were lost; the notified() below would wait until the timeout
// Control:  a second notified() with no permit left does wait (times out)
#[tokio::test(start_paused = true)]
#[ignore = "vendor test"]
async fn tokio_notify_must_keep_a_permit_for_a_later_waiter() {
    let notify = tokio::sync::Notify::new();
    notify.notify_one();
    let first = tokio::time::timeout(Duration::from_millis(10), notify.notified()).await;
    assert!(
        first.is_ok(),
        "the stored permit should complete notified() at once"
    );
    let second = tokio::time::timeout(Duration::from_millis(10), notify.notified()).await;
    assert!(
        second.is_err(),
        "no permit is left, so notified() should wait"
    );
}

struct SetOnDrop(Arc<AtomicBool>);
impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The F3 fix (run()'s cleanup guard) relies on this when a runtime shuts down under a running
/// connection.
// Claim:    shutting down a runtime drops the futures of its unfinished tasks, running their destructors
// Fails if: shutdown leaked unfinished tasks without dropping them; the flag would stay false
// Control:  the task is first observed running, so it is unfinished at shutdown
#[test]
#[ignore = "vendor test"]
fn tokio_runtime_shutdown_must_run_destructors_of_unfinished_tasks() {
    let dropped = Arc::new(AtomicBool::new(false));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let guard = SetOnDrop(dropped.clone());
    runtime.spawn(async move {
        let _guard = guard;
        started_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    started_rx.recv().unwrap();
    assert!(!dropped.load(Ordering::SeqCst));

    drop(runtime);
    assert!(dropped.load(Ordering::SeqCst));
}

/// The F4 fix (the drain task closes its connection when it ends) relies on this when the
/// on_message callback panics.
// Claim:    a panic in a spawned task ends only that task, runs the destructors of its locals, and its JoinHandle reports the panic
// Fails if: the panic leaked the task's locals (flag stays false), was reported as a normal end or a cancellation, or took down the runtime (the later task would not run)
// Control:  a task spawned after the panic still runs to completion
#[tokio::test]
#[ignore = "vendor test"]
async fn tokio_panicking_task_must_run_destructors_and_leave_the_runtime_running() {
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = SetOnDrop(dropped.clone());
    let task = tokio::spawn(async move {
        let _guard = guard;
        panic!("callback panicked");
    });
    assert!(task.await.unwrap_err().is_panic());
    assert!(dropped.load(Ordering::SeqCst));

    assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
}

/// The F2 fix (the Ruby binding waits for the receiver lock inside its cancellable select!) relies
/// on this.
// Claim:    dropping a tokio Mutex lock() future that is still waiting neither acquires the lock nor blocks later lockers
// Fails if: a cancelled waiter kept its place or took the lock when it was released; the later lock() would wait until the timeout
// Control:  the cancelled lock() was really waiting (it timed out while the lock was held)
#[tokio::test(start_paused = true)]
#[ignore = "vendor test"]
async fn tokio_mutex_lock_cancelled_while_waiting_must_not_block_later_lockers() {
    let mutex = tokio::sync::Mutex::new(());
    let held = mutex.lock().await;

    let cancelled = tokio::time::timeout(Duration::from_millis(10), mutex.lock()).await;
    assert!(
        cancelled.is_err(),
        "lock() should still be waiting when the timer fires"
    );

    drop(held);
    let later = tokio::time::timeout(Duration::from_millis(10), mutex.lock()).await;
    assert!(later.is_ok(), "the cancelled waiter blocked a later lock()");
}
