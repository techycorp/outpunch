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
/// interrupts the thread (Thread#kill, Thread#raise, Timeout); it returns `None` when
/// it stopped early because of that.
///
/// Returns `Ok(None)` when the call was cancelled and Ruby had no exception pending.
pub fn without_gvl<F, R>(ruby: &Ruby, func: F) -> Result<Option<R>, Error>
where
    F: FnOnce(Arc<Notify>) -> Option<R>,
{
    let cancel = Arc::new(Notify::new());
    let mut state = Call {
        func: Some({
            let cancel = cancel.clone();
            move || func(cancel)
        }),
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
        Some(Ok(Some(value))) => Ok(Some(value)),
        Some(Ok(None)) | None => {
            ruby.thread_check_ints()?;
            Ok(None)
        }
        Some(Err(payload)) => panic::resume_unwind(payload),
    }
}
