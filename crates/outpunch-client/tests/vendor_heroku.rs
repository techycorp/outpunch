//! Facts about Heroku's router that the heartbeat relies on (R30, R31 in
//! spec/REQUIREMENTS.md). Observes a real app on Heroku; does not test our code. Listed in
//! vendor-claims.toml.
//!
//! Run with: OUTPUNCH_HEROKU_WS_URL=wss://<app>.herokuapp.com/ws \
//!   cargo test -p outpunch-client --test vendor_heroku -- --ignored
//! Needs only the URL of an outpunch server on Heroku; no secret (the ping is sent before auth).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio_tungstenite::tungstenite::Message;

/// The same TLS setup as the client (crates/outpunch-client/src/tls.rs).
fn connector() -> tokio_tungstenite::Connector {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_platform_verifier()
    .unwrap()
    .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}

// Claim:    a WebSocket ping sent through Heroku's router is answered with a pong carrying its payload
// Fails if: the router dropped the ping or the pong (no pong within 5 s), or answered with a
//           different payload
// Control:  the payload is chosen by this test, so the pong answers this ping
// Not shown: whether the dyno or the router produced the pong (see vendor-claims.toml)
#[tokio::test]
#[ignore = "vendor test"]
async fn heroku_must_answer_a_websocket_ping_with_its_pong() {
    let Ok(url) = std::env::var("OUTPUNCH_HEROKU_WS_URL") else {
        eprintln!("skipped: OUTPUNCH_HEROKU_WS_URL is not set");
        return;
    };
    let (mut ws, _) =
        tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector()))
            .await
            .unwrap();
    let payload = format!("outpunch-vendor-{}", std::process::id());
    ws.send(Message::Ping(payload.clone().into_bytes().into()))
        .await
        .unwrap();

    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Pong(data) = msg {
                return Some(data);
            }
        }
        None
    })
    .await
    .expect("no pong within 5 s");
    assert_eq!(reply.as_deref(), Some(payload.as_bytes()));
    let _ = ws.close(None).await;
}
