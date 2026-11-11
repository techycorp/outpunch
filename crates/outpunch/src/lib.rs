/// Records a trace event when built with the `trace` feature; expands to nothing otherwise.
#[cfg(feature = "trace")]
macro_rules! trace_event {
    ($($event:tt)+) => {
        $crate::trace::emit(serde_json::json!($($event)+))
    };
}

#[cfg(not(feature = "trace"))]
macro_rules! trace_event {
    ($($event:tt)+) => {};
}

#[cfg(feature = "stderr-logging")]
pub mod logging;
pub mod protocol;
pub mod server;
#[cfg(feature = "trace")]
mod trace;
