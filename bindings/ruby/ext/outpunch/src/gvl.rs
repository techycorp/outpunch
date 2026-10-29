use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use magnus::{Error, Ruby};
use tokio::sync::Notify;

struct Call<F, R> {
    func: Option<F>,
    result: Option<std::thread::Result<R>>,
}

unsafe extern "C" fn call<F: FnOnce() -> R, R>(data: *mut c_void) -> *mut c_void {
    let call = unsafe { &mut *(data as *mut Call<F, R>) };
    if let Some(func) = call.func.take() {
        call.result = Some(panic::catch_unwind(AssertUnwindSafe(func)));
    }
    std::ptr::null_mut()
}

fn trampoline<F: FnOnce() -> R, R>(
    _: &Call<F, R>,
) -> unsafe extern "C" fn(*mut c_void) -> *mut c_void {
    call::<F, R>
}

unsafe extern "C" fn unblock(data: *mut c_void) {
    let cancel = unsafe { &*(data as *const Notify) };
    cancel.notify_one();
}

/// Run `func` with the GVL released. `func` receives a `Notify` that fires when Ruby
/// interrupts the thread (Thread#raise, Thread#kill, Timeout, Thread#wakeup, signals); it
/// returns `None` when it stopped early because of that.
///
/// `rb_thread_call_without_gvl2` does not call `func` at all while an interrupt is pending,
/// and the VM timer makes one pending whenever threads compete for the GVL. So when `func` did
/// not run, the interrupt is handled and `func` is called again.
///
/// Returns `Err` when Ruby has an exception to raise, `Ok(None)` when `func` was cancelled by an
/// interrupt that raised nothing (the caller decides whether to try again), `Ok(Some)` otherwise.
pub fn without_gvl<F, R>(ruby: &Ruby, mut func: F) -> Result<Option<R>, Error>
where
    F: FnMut(Arc<Notify>) -> Option<R>,
{
    loop {
        let cancel = Arc::new(Notify::new());
        let mut state = Call {
            func: Some(|| func(cancel.clone())),
            result: None,
        };

        unsafe {
            rb_sys::rb_thread_call_without_gvl2(
                Some(trampoline(&state)),
                &mut state as *mut _ as *mut c_void,
                Some(unblock),
                Arc::as_ptr(&cancel) as *mut c_void,
            );
        }

        match state.result {
            Some(Ok(Some(value))) => return Ok(Some(value)),
            Some(Ok(None)) => {
                ruby.thread_check_ints()?;
                return Ok(None);
            }
            Some(Err(payload)) => panic::resume_unwind(payload),
            None => ruby.thread_check_ints()?,
        }
    }
}
