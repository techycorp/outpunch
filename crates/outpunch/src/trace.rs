//! Execution traces for TLA+ trace validation: one NDJSON event per step of `spec/Tunnel.tla`.
//! Compiled only with the `trace` feature. Events are appended to the file named by the
//! `OUTPUNCH_TRACE` environment variable; without it, nothing is written.
//!
//! Event order must match the order in which steps take effect, so traces are only
//! meaningful from a single-threaded runtime (as in `#[tokio::test]`).

use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use crate::protocol::{self, Message};

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub(crate) fn emit(event: serde_json::Value) {
    let sink = SINK.get_or_init(|| {
        let path = std::env::var_os("OUTPUNCH_TRACE")?;
        let file = File::options().create(true).append(true).open(path).ok()?;
        Some(Mutex::new(file))
    });
    if let Some(file) = sink {
        let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(file, "{event}");
    }
}

/// The model's view of a pushed message: an auth (service, whether the token matches the
/// secret), a response (request id), or anything else.
pub(crate) fn message(raw: &str, secret: &str) -> serde_json::Value {
    match protocol::parse_message(raw) {
        Ok(Message::Auth(auth)) => serde_json::json!({
            "type": "auth",
            "svc": auth.service,
            // Plain comparison, so a broken constant_time_eq cannot validate its own trace.
            "token_ok": auth.token == secret,
        }),
        Ok(Message::Response(resp)) => serde_json::json!({"type": "resp", "req": resp.request_id}),
        _ => serde_json::json!({"type": "junk"}),
    }
}
