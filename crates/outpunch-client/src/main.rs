use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::{self, AuthMessage, Message, TunnelRequest, TunnelResponse};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Parser)]
#[command(name = "outpunch-client", about = "Outpunch tunnel client")]
struct Args {
    /// WebSocket URL of the outpunch server
    #[arg(
        long,
        env = "OUTPUNCH_SERVER_URL",
        default_value = "ws://localhost:3000/ws"
    )]
    server_url: String,

    /// Shared secret for authentication
    #[arg(long, env = "OUTPUNCH_SECRET")]
    secret: String,

    /// Service name to register for
    #[arg(long, env = "OUTPUNCH_SERVICE")]
    service: String,

    /// Local URL to forward requests to
    #[arg(
        long,
        env = "OUTPUNCH_FORWARD_TO",
        default_value = "http://localhost:8080"
    )]
    forward_to: String,

    /// Seconds to wait before reconnecting
    #[arg(long, env = "OUTPUNCH_RECONNECT_DELAY", default_value = "5")]
    reconnect_delay: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let reconnect_delay = Duration::from_secs(args.reconnect_delay);

    eprintln!("outpunch-client");
    eprintln!("  server:     {}", args.server_url);
    eprintln!("  service:    {}", args.service);
    eprintln!("  forward_to: {}", args.forward_to);

    loop {
        match run_connection(&args).await {
            Ok(()) => eprintln!("connection closed"),
            Err(e) => eprintln!("error: {e}"),
        }
        eprintln!("reconnecting in {}s...", reconnect_delay.as_secs());
        tokio::time::sleep(reconnect_delay).await;
    }
}

async fn run_connection(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("connecting to {}...", args.server_url);
    let (ws_stream, _) = connect_async(&args.server_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // Send auth
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: args.secret.clone(),
        service: args.service.clone(),
    };
    ws_sink
        .send(WsMessage::text(serde_json::to_string(&auth)?))
        .await?;

    // Wait for auth response
    let auth_resp = ws_stream
        .next()
        .await
        .ok_or("connection closed before auth response")??;

    let text = match auth_resp {
        WsMessage::Text(t) => t,
        _ => return Err("unexpected WS message type".into()),
    };

    match protocol::parse_message(&text)? {
        Message::AuthOk(_) => eprintln!("authenticated"),
        Message::AuthError(e) => return Err(format!("auth rejected: {}", e.message).into()),
        _ => return Err("unexpected message during auth".into()),
    }

    // Listen for requests
    let http_client = reqwest::Client::new();

    while let Some(msg) = ws_stream.next().await {
        let text = match msg? {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => break,
            _ => continue,
        };

        match protocol::parse_message(&text) {
            Ok(Message::Request(req)) => {
                eprintln!("[{:.8}] {} /{}", req.request_id, req.method, req.path);
                let response = forward_request(&http_client, &args.forward_to, &req).await;
                eprintln!("[{:.8}] -> {}", req.request_id, response.status);

                ws_sink
                    .send(WsMessage::text(protocol::serialize_response(&response)))
                    .await?;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }

    Ok(())
}

async fn forward_request(
    client: &reqwest::Client,
    forward_to: &str,
    req: &TunnelRequest,
) -> TunnelResponse {
    let base = forward_to.trim_end_matches('/');
    let url = if req.query.is_empty() {
        format!("{base}/{}", req.path)
    } else {
        let qs: Vec<String> = req.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{base}/{}?{}", req.path, qs.join("&"))
    };

    let mut http_req = client.request(req.method.parse().unwrap_or(reqwest::Method::GET), &url);

    for (key, value) in &req.headers {
        http_req = http_req.header(key.as_str(), value.as_str());
    }

    if let Some(body) = &req.body {
        http_req = http_req.body(body.clone());
    }

    match http_req.timeout(Duration::from_secs(25)).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let headers: HashMap<String, String> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                .collect();

            let body_bytes = resp.bytes().await.unwrap_or_default();

            TunnelResponse {
                msg_type: "response".to_string(),
                request_id: req.request_id.clone(),
                status,
                headers,
                body: Some(BASE64.encode(&body_bytes)),
                body_encoding: Some("base64".to_string()),
            }
        }
        Err(e) if e.is_timeout() => {
            protocol::error_response(&req.request_id, 504, "local service timeout")
        }
        Err(e) => {
            protocol::error_response(&req.request_id, 502, &format!("local service error: {e}"))
        }
    }
}
