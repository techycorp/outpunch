use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::RawQuery;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::IncomingRequest;
use outpunch::server::OutpunchServer;
use tokio::sync::mpsc;

/// Build an axum Router with outpunch tunnel routes.
pub fn router(server: OutpunchServer) -> Router {
    let state = Arc::new(server);

    Router::new()
        .route("/ws", get(ws_handler))
        .route("/tunnel/{service}/{*path}", any(tunnel_handler))
        .route("/tunnel/{service}", any(tunnel_handler_no_path))
        .with_state(state)
}

async fn tunnel_handler(
    State(server): State<Arc<OutpunchServer>>,
    Path((service, path)): Path<(String, String)>,
    method: Method,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response {
    handle_tunnel(server, service, path, method, raw_query, headers, body).await
}

async fn tunnel_handler_no_path(
    State(server): State<Arc<OutpunchServer>>,
    Path(service): Path<String>,
    method: Method,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response {
    handle_tunnel(
        server,
        service,
        String::new(),
        method,
        raw_query,
        headers,
        body,
    )
    .await
}

async fn handle_tunnel(
    server: Arc<OutpunchServer>,
    service: String,
    path: String,
    method: Method,
    raw_query: Option<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let body_bytes = match axum::body::to_bytes(body, server.max_body_size()).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "request body too large").into_response();
        }
    };

    let body_str = if body_bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&body_bytes).into_owned())
    };

    let query = parse_query(raw_query.as_deref());

    let incoming = IncomingRequest {
        service,
        method: method.to_string(),
        path,
        query,
        headers: extract_headers(&headers),
        body: body_str,
    };

    let resp = server.handle_request(incoming).await;
    tunnel_response_to_axum(resp)
}

async fn ws_handler(State(server): State<Arc<OutpunchServer>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_ws(server, socket))
}

/// Bridge a WebSocket to the core's channel interface.
async fn handle_ws(server: Arc<OutpunchServer>, socket: WebSocket) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Channels between bridge and core
    let (incoming_tx, incoming_rx) = mpsc::channel::<String>(64);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(64);

    // Bridge: WS stream → incoming_tx
    let read_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                WsMessage::Text(text) => {
                    if incoming_tx.send(text.to_string()).await.is_err() {
                        break;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });

    // Bridge: outgoing_rx → WS sink
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = outgoing_rx.recv().await {
            if ws_sink.send(WsMessage::text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Core handles the connection.
    // When this returns, outgoing_tx is dropped, signaling the write task to end.
    server.handle_connection(incoming_rx, outgoing_tx).await;

    // Give the write task time to flush remaining messages before closing
    let _ = tokio::time::timeout(Duration::from_millis(100), write_handle).await;
    read_handle.abort();
}

fn parse_query(raw: Option<&str>) -> HashMap<String, String> {
    let Some(qs) = raw else {
        return HashMap::new();
    };

    qs.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn extract_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let skip = ["host", "connection", "upgrade", "transfer-encoding"];

    headers
        .iter()
        .filter(|(name, _)| !skip.contains(&name.as_str()))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

fn tunnel_response_to_axum(resp: outpunch::protocol::TunnelResponse) -> Response {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let body_bytes = match (resp.body, resp.body_encoding.as_deref()) {
        (Some(encoded), Some("base64")) => BASE64
            .decode(&encoded)
            .unwrap_or_else(|_| encoded.into_bytes()),
        (Some(plain), _) => plain.into_bytes(),
        (None, _) => Vec::new(),
    };

    let mut builder = Response::builder().status(status);

    for (key, value) in &resp.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }

    builder.body(Body::from(body_bytes)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("internal error"))
            .unwrap()
    })
}
