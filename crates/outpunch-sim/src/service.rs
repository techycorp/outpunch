//! The stand-in local service the client forwards to. Each request says how to answer through
//! headers the simulator adds (they travel through the tunnel like any other header):
//! `x-sim-status`, `x-sim-delay-ms`, `x-sim-body`. Without them it answers 200 "ok" at once.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;

use crate::logs::Log;

pub const STATUS: &str = "x-sim-status";
pub const DELAY_MS: &str = "x-sim-delay-ms";
pub const BODY: &str = "x-sim-body";

/// Serves on a free local port; returns the port.
pub async fn start(log: Log) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let app = Router::new().fallback(move |method: Method, uri: Uri, headers: HeaderMap| {
        let log = log.clone();
        async move { answer(&log, method, uri, headers).await }
    });
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(port)
}

async fn answer(log: &Log, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let status = header(STATUS)
        .and_then(|s| s.parse().ok())
        .and_then(|s| StatusCode::from_u16(s).ok())
        .unwrap_or(StatusCode::OK);
    let delay_ms: u64 = header(DELAY_MS).and_then(|d| d.parse().ok()).unwrap_or(0);
    let body = header(BODY).unwrap_or("ok").to_string();

    log.info(&format!(
        "{method} {uri} (answering {} after {delay_ms} ms)",
        status.as_u16()
    ));
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    Response::builder()
        .status(status)
        .body(Body::from(body))
        .expect("status and body are valid")
}
