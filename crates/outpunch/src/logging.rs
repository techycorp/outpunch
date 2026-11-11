//! Printing log events to stderr in the docs/logging.md format, for binaries and bindings (the
//! `stderr-logging` feature). Library code only emits events through `tracing`; nothing here runs
//! unless a binary or binding calls [`init_stderr`].
//!
//! ```text
//! 22:12:28.717511Z  INFO outpunch_client: request req=1df08b44 method=GET path=/hello status=200
//! ```

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

/// UTC time of day with microseconds, `HH:MM:SS.ffffffZ`: fixed width, no date (the platform's
/// log lines, or the file, carry the date).
pub fn time_of_day(at: SystemTime) -> String {
    let since_epoch = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:06}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        since_epoch.subsec_micros()
    )
}

/// The formatter's clock: [`time_of_day`] of now.
struct TimeOfDay;

impl FormatTime for TimeOfDay {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        w.write_str(&time_of_day(SystemTime::now()))
    }
}

/// Prints log events to stderr: time of day, level, target, message, `key=value` fields. Filtered
/// by `RUST_LOG` or, when that is unset, by `default_filter` (an `EnvFilter` directive such as
/// `info`). Color only when stderr is a terminal. Does nothing if the process already has a
/// subscriber.
pub fn init_stderr(default_filter: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_timer(TimeOfDay)
        .with_env_filter(filter)
        .try_init();
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
