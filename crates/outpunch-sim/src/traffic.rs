//! The traffic file: one JSON object per line, each a request or an event, scheduled by `at_ms`
//! (milliseconds after the tunnel is up). Blank lines and lines starting with `#` are skipped.
//!
//! ```text
//! {"at_ms": 0, "method": "GET", "path": "/hello"}
//! {"at_ms": 50, "method": "POST", "path": "/upload", "body": "hi", "reply": {"status": 201, "delay_ms": 200}}
//! {"at_ms": 1000, "event": "client_pause", "ms": 5000}
//! ```

use std::collections::HashMap;

use serde::Deserialize;

/// What the stand-in local service answers for one request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub reply: Reply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// SIGSTOP the client for `ms`, then SIGCONT: it neither pings nor reads meanwhile.
    ClientPause { ms: u64 },
    /// Kill the client (no close frame) and start a new one.
    ClientRestart,
    /// Kill the server and start a new one on the same port.
    ServerRestart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Request(Request),
    Event(Event),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// 1-based line number in the traffic file.
    pub line: usize,
    pub at_ms: u64,
    pub action: Action,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLine {
    at_ms: u64,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    reply: Reply,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    ms: Option<u64>,
}

/// Parses a whole traffic file into steps sorted by `at_ms` (stable for equal times).
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = text
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(n, line)| {
            parse_line(line)
                .map_err(|e| format!("line {n}: {e}"))
                .map(|(at_ms, action)| Step {
                    line: n,
                    at_ms,
                    action,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    steps.sort_by_key(|step| step.at_ms);
    Ok(steps)
}

fn parse_line(line: &str) -> Result<(u64, Action), String> {
    let raw: RawLine = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let action = match raw.event.as_deref() {
        None => Action::Request(request(
            raw.method,
            raw.path,
            raw.headers,
            raw.body,
            raw.reply,
        )?),
        Some("client_pause") => Action::Event(Event::ClientPause {
            ms: raw.ms.ok_or("client_pause needs \"ms\"")?,
        }),
        Some("client_restart") => Action::Event(Event::ClientRestart),
        Some("server_restart") => Action::Event(Event::ServerRestart),
        Some(other) => {
            return Err(format!(
                "unknown event {other:?} (client_pause, client_restart, server_restart)"
            ));
        }
    };
    Ok((raw.at_ms, action))
}

fn request(
    method: Option<String>,
    path: Option<String>,
    headers: HashMap<String, String>,
    body: Option<String>,
    reply: Reply,
) -> Result<Request, String> {
    let path = path.ok_or("a request needs \"path\"")?;
    if !path.starts_with('/') {
        return Err(format!("path {path:?} must start with /"));
    }
    Ok(Request {
        method: method.unwrap_or_else(|| "GET".to_string()).to_uppercase(),
        path,
        headers,
        body,
        reply,
    })
}

#[cfg(test)]
#[path = "traffic_tests.rs"]
mod tests;
