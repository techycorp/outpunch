use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::{self, AuthMessage, Message, TunnelRequest, TunnelResponse};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_url: String,
    pub secret: String,
    pub service: String,
    pub forward_to: String,
    pub reconnect_delay: Duration,
    pub request_timeout: Duration,
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
    let (ws_stream, _) = connect_async(&config.server_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    authenticate(&mut ws_sink, &mut ws_stream, config).await?;

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
                let response = forward_request(&http_client, config, &req).await;
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
