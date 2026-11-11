use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::{self, AuthMessage, Message, TunnelRequest, TunnelResponse, short_id};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{error, info, warn};

mod tls;

/// Why a connection ended, for the errors a caller may want to tell apart. Returned inside the
/// boxed error of [`run_connection`]; other failures (protocol errors, resets) are left as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    /// The server could not be reached or the WebSocket handshake failed.
    Connect(String),
    /// The server refused the secret or service; its message.
    AuthRejected(String),
    /// Nothing, not even a pong, arrived from the server within the idle timeout.
    ServerSilent(Duration),
    /// A write to the server did not finish within the idle timeout.
    WriteTimedOut(Duration),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "could not connect: {e}"),
            Self::AuthRejected(message) => write!(f, "auth rejected: {message}"),
            Self::ServerSilent(idle) => {
                write!(f, "no message from the server in {}s", idle.as_secs())
            }
            Self::WriteTimedOut(limit) => {
                write!(f, "a write to the server took over {}s", limit.as_secs())
            }
        }
    }
}

impl std::error::Error for ConnectionError {}

/// Prints outpunch's log events to stderr (docs/logging.md, Format); see
/// [`outpunch::logging::init_stderr`]. For binaries and bindings; the library itself never calls it.
pub fn init_stderr_logging(default_filter: &str) {
    outpunch::logging::init_stderr(default_filter);
}

/// A server URL as logged: without any query string.
fn loggable_url(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

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
        let started = Instant::now();
        let result = run_connection(config).await;
        log_end(&result, started.elapsed());
        info!(delay_s = config.reconnect_delay.as_secs(), "reconnecting");
        tokio::time::sleep(config.reconnect_delay).await;
    }
}

/// Logs how a connection attempt ended (docs/logging.md, client events).
fn log_end(result: &Result<(), Box<dyn std::error::Error + Send + Sync>>, duration: Duration) {
    let duration_s = duration.as_secs();
    let Err(e) = result else {
        info!(reason = %"server_closed", duration_s, "connection closed");
        return;
    };
    match e.downcast_ref::<ConnectionError>() {
        Some(ConnectionError::Connect(e)) => warn!(error = ?e, "connect failed"),
        Some(ConnectionError::AuthRejected(_)) => {
            info!(reason = %"auth_rejected", duration_s, "connection closed")
        }
        Some(ConnectionError::ServerSilent(_)) => {
            info!(reason = %"idle", duration_s, "connection closed")
        }
        Some(ConnectionError::WriteTimedOut(_)) => {
            info!(reason = %"write_timeout", duration_s, "connection closed")
        }
        None => warn!(
            reason = %"error",
            duration_s,
            error = ?e.to_string(),
            "connection closed"
        ),
    }
}

/// Run a single connection attempt. Returns when the connection ends.
pub async fn run_connection(
    config: &ClientConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        server = %loggable_url(&config.server_url),
        service = %config.service,
        "connecting"
    );
    let (ws_stream, _) =
        connect_async_tls_with_config(&config.server_url, None, false, Some(tls::connector()?))
            .await
            .map_err(|e| ConnectionError::Connect(e.to_string()))?;
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
                        let http_client = http_client.clone();
                        let config = config.clone();
                        let response_tx = response_tx.clone();
                        in_flight.spawn(async move {
                            let started = Instant::now();
                            let response = forward_request(&http_client, &config, &req).await;
                            info!(
                                req = %short_id(&req.request_id),
                                method = %req.method,
                                path = %format!("/{}", req.path),
                                status = response.status,
                                duration_ms = started.elapsed().as_millis() as u64,
                                "request"
                            );
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
                warn!(
                    idle_s = config.idle_timeout.as_secs(),
                    "no message from the server; reconnecting"
                );
                return Err(ConnectionError::ServerSilent(config.idle_timeout).into());
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
        Err(_) => {
            warn!(
                wait_s = limit.as_secs(),
                "write to the server timed out; reconnecting"
            );
            Err(ConnectionError::WriteTimedOut(limit).into())
        }
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
            info!(service = %config.service, "authenticated");
            Ok(())
        }
        Message::AuthError(e) => {
            error!(reason = ?e.message, "auth rejected");
            Err(ConnectionError::AuthRejected(e.message).into())
        }
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
            warn!(
                req = %short_id(&req.request_id),
                status = 504,
                error = %"timeout",
                "local service failed"
            );
            protocol::error_response(&req.request_id, 504, "local service timeout")
        }
        Err(e) => {
            let response = protocol::error_response(
                &req.request_id,
                502,
                &format!("local service error: {e}"),
            );
            // without_url: the URL can carry the request's query string, which is never logged.
            warn!(
                req = %short_id(&req.request_id),
                status = 502,
                error = ?e.without_url().to_string(),
                "local service failed"
            );
            response
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
