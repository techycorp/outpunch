//! Facts about axum's WebSocket support (tungstenite underneath) and the tokio-tungstenite
//! client that the heartbeat relies on (R30, R31 in spec/REQUIREMENTS.md). Each test observes
//! the real libraries; none tests our code. Listed in vendor-claims.toml.
//!
//! Run with: cargo test -p outpunch-axum --test vendor_axum -- --ignored

use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message as AxumMessage, WebSocketUpgrade};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as ClientMessage;

/// Serves one WebSocket endpoint whose handler reads forever, reporting each message it sees.
async fn reading_server() -> (String, mpsc::UnboundedReceiver<AxumMessage>) {
    let (seen_tx, seen_rx) = mpsc::unbounded_channel();
    let app = Router::new().route(
        "/ws",
        get(move |ws: WebSocketUpgrade| {
            let seen_tx = seen_tx.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    while let Some(Ok(msg)) = socket.recv().await {
                        let _ = seen_tx.send(msg);
                    }
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("ws://{addr}/ws"), seen_rx)
}

/// The axum adapter's read task relies on this to count a client ping as a sign of life.
// Claim:    axum delivers a client's WebSocket ping frame to the handler as Message::Ping
// Fails if: axum answered pings internally without surfacing them; the handler would see only
//           the text frame
// Control:  a text frame sent after the ping is delivered too, so the handler was reading
#[tokio::test]
#[ignore = "vendor test"]
async fn axum_must_deliver_client_ping_frames_to_the_handler() {
    let (url, mut seen) = reading_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(ClientMessage::Ping(b"p".to_vec().into()))
        .await
        .unwrap();
    ws.send(ClientMessage::text("after")).await.unwrap();

    let first = tokio::time::timeout(Duration::from_secs(2), seen.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(2), seen.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(first, AxumMessage::Ping(ref p) if p.as_ref() == b"p"),
        "{first:?}"
    );
    assert!(
        matches!(second, AxumMessage::Text(ref t) if t.as_str() == "after"),
        "{second:?}"
    );
}

/// The client's idle check relies on this: the pong is what proves a quiet server is alive.
// Claim:    while an axum handler is reading, a client ping is answered with a pong that the tokio-tungstenite client yields as Message::Pong
// Fails if: no pong came back (the client would see nothing within the timeout) or the client
//           consumed it internally
// Control:  the pong carries the ping's payload
#[tokio::test]
#[ignore = "vendor test"]
async fn axum_must_answer_a_client_ping_with_a_pong_the_client_sees() {
    let (url, _seen) = reading_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(ClientMessage::Ping(b"p".to_vec().into()))
        .await
        .unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("no reply to the ping")
        .unwrap()
        .unwrap();
    assert!(
        matches!(reply, ClientMessage::Pong(ref p) if p.as_ref() == b"p"),
        "{reply:?}"
    );
}
