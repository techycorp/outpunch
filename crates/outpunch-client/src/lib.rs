use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::{self, AuthMessage, Message, TunnelRequest, TunnelResponse};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::Message as WsMessage;

mod tls;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_url: String,
    pub secret: String,
    pub service: String,
    pub forward_to: String,
    pub reconnect_delay: Duration,
    pub request_timeout: Duration,
    /// How often the client sends a WebSocket ping. The server counts pings as signs of
    /// life, so this must stay well under the server's client_idle_timeout (90 s by default).
    pub ping_interval: Duration,
    /// How long the client waits without receiving anything (a pong or any other frame)
    /// before it treats the connection as dead and reconnects. Must be longer than
    /// ping_interval.
    pub idle_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: "ws://localhost:3000/ws".to_string(),
            secret: String::new(),
            forward_to: "http://localhost:8080".to_string(),
            service: String::new(),
            reconnect_delay: Duration::from_secs(5),
            request_timeout: Duration::from_secs(25),
            ping_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(75),
        }
    }
}

/// Run the tunnel client forever, reconnecting on failure.
pub async fn run(config: &ClientConfig) {
    loop {
        match run_connection(config).await {
            Ok(()) => eprintln!("connection closed"),
            Err(e) => eprintln!("error: {e}"),
        }
        eprintln!("reconnecting in {}s...", config.reconnect_delay.as_secs());
        tokio::time::sleep(config.reconnect_delay).await;
    }
}

/// Run a single connection attempt. Returns when the connection ends.
pub async fn run_connection(
    config: &ClientConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("connecting to {}...", config.server_url);
    let (ws_stream, _) =
        connect_async_tls_with_config(&config.server_url, None, false, Some(tls::connector()?))
            .await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    authenticate(&mut ws_sink, &mut ws_stream, config).await?;

    let http_client = reqwest::Client::new();
    let mut ping_interval = tokio::time::interval(config.ping_interval);
    ping_interval.tick().await; // discard immediate first tick
    // Anything from the server (a pong answering our ping, a request) proves the connection
    // is alive. Without this, a half-open connection is only noticed when TCP gives up on
    // the unacknowledged pings, which takes about 15 minutes on Linux.
    let mut last_heard = tokio::time::Instant::now();

    // Requests are forwarded concurrently; their responses come back on this channel so
    // only this loop writes to the WebSocket.
    let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel::<TunnelResponse>();
    let mut in_flight = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            Some(response) = response_rx.recv() => {
                eprintln!("[{:.8}] -> {}", response.request_id, response.status);
                let text = WsMessage::text(protocol::serialize_response(&response));
                send_within(&mut ws_sink, text, config.idle_timeout).await?;
            }
            Some(_) = in_flight.join_next(), if !in_flight.is_empty() => {}
            msg = ws_stream.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    _ => break,
                };
                last_heard = tokio::time::Instant::now();
                let text = match msg {
                    WsMessage::Text(t) => t,
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                match protocol::parse_message(&text) {
                    Ok(Message::Request(req)) => {
                        eprintln!("[{:.8}] {} /{}", req.request_id, req.method, req.path);
                        let http_client = http_client.clone();
                        let config = config.clone();
                        let response_tx = response_tx.clone();
                        in_flight.spawn(async move {
                            let response = forward_request(&http_client, &config, &req).await;
                            let _ = response_tx.send(response);
                        });
                    }
                    Ok(_) => continue,
                    Err(_) => continue,
                }
            }
            _ = ping_interval.tick() => {
                send_within(&mut ws_sink, WsMessage::Ping(vec![].into()), config.idle_timeout).await?;
            }
            _ = tokio::time::sleep_until(last_heard + config.idle_timeout) => {
                return Err(format!(
                    "no message from the server in {}s; reconnecting",
                    config.idle_timeout.as_secs()
                )
                .into());
            }
        }
    }

    Ok(())
}

/// Sends a frame, failing if the server does not take it within `limit`: a write blocked on a
/// dead connection must not stop the idle check.
async fn send_within<S>(
    ws_sink: &mut S,
    msg: WsMessage,
    limit: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: futures_util::Sink<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    match tokio::time::timeout(limit, ws_sink.send(msg)).await {
        Ok(sent) => Ok(sent?),
        Err(_) => Err(format!("a write to the server took over {}s", limit.as_secs()).into()),
    }
}

async fn authenticate(
    ws_sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMessage,
    >,
    ws_stream: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    config: &ClientConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: config.secret.clone(),
        service: config.service.clone(),
    };
    ws_sink
        .send(WsMessage::text(serde_json::to_string(&auth)?))
        .await?;

    let auth_resp = ws_stream
        .next()
        .await
        .ok_or("connection closed before auth response")??;

    let text = match auth_resp {
        WsMessage::Text(t) => t,
        _ => return Err("unexpected WS message type".into()),
    };

    match protocol::parse_message(&text)? {
        Message::AuthOk(_) => {
            eprintln!("authenticated");
            Ok(())
        }
        Message::AuthError(e) => Err(format!("auth rejected: {}", e.message).into()),
        _ => Err("unexpected message during auth".into()),
    }
}

pub fn forward_url(forward_to: &str, req: &TunnelRequest) -> String {
    let base = forward_to.trim_end_matches('/');
    if req.query.is_empty() {
        format!("{base}/{}", req.path)
    } else {
        let qs: Vec<String> = req.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{base}/{}?{}", req.path, qs.join("&"))
    }
}

pub async fn forward_request(
    client: &reqwest::Client,
    config: &ClientConfig,
    req: &TunnelRequest,
) -> TunnelResponse {
    let url = forward_url(&config.forward_to, req);

    let mut http_req = client.request(req.method.parse().unwrap_or(reqwest::Method::GET), &url);

    for (key, value) in &req.headers {
        http_req = http_req.header(key.as_str(), value.as_str());
    }

    if let Some(body) = &req.body {
        http_req = http_req.body(body.clone());
    }

    match http_req.timeout(config.request_timeout).send().await {
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
