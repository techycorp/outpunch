use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, oneshot};

use crate::protocol::{self, IncomingRequest, Message, TunnelResponse};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub secret: String,
    pub timeout: Duration,
    pub max_body_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            timeout: Duration::from_secs(25),
            max_body_size: 10 * 1024 * 1024, // 10MB
        }
    }
}

/// Represents an active client connection.
struct Connection {
    outgoing_tx: mpsc::Sender<String>,
}

/// Shared state for the outpunch server.
struct ServerState {
    config: ServerConfig,
    /// service name → active connection
    services: HashMap<String, Connection>,
    /// request_id → oneshot sender to deliver the response
    pending: HashMap<String, oneshot::Sender<TunnelResponse>>,
}

#[derive(Clone)]
pub struct OutpunchServer {
    state: Arc<Mutex<ServerState>>,
    max_body_size: usize,
}

impl OutpunchServer {
    pub fn new(config: ServerConfig) -> Self {
        let max_body_size = config.max_body_size;
        Self {
            state: Arc::new(Mutex::new(ServerState {
                config,
                services: HashMap::new(),
                pending: HashMap::new(),
            })),
            max_body_size,
        }
    }

    pub fn max_body_size(&self) -> usize {
        self.max_body_size
    }

    /// Handle an incoming HTTP tunnel request. Returns a TunnelResponse.
    /// Called by the adapter when a request hits /tunnel/*path.
    pub async fn handle_request(&self, incoming: IncomingRequest) -> TunnelResponse {
        let tunnel_req = protocol::build_tunnel_request(&incoming);
        let request_id = tunnel_req.request_id.clone();
        let json = protocol::serialize_request(&tunnel_req);

        let (resp_tx, resp_rx) = oneshot::channel();
        let timeout;

        {
            let mut state = self.state.lock().await;
            timeout = state.config.timeout;

            let Some(conn) = state.services.get(&incoming.service) else {
                return protocol::error_response(
                    &request_id,
                    502,
                    "no client connected for service",
                );
            };

            if conn.outgoing_tx.send(json).await.is_err() {
                return protocol::error_response(&request_id, 502, "client connection lost");
            }

            state.pending.insert(request_id.clone(), resp_tx);
        }

        match tokio::time::timeout(timeout, resp_rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                // Sender dropped — client disconnected
                self.state.lock().await.pending.remove(&request_id);
                protocol::error_response(&request_id, 502, "client disconnected")
            }
            Err(_) => {
                // Timeout
                self.state.lock().await.pending.remove(&request_id);
                protocol::error_response(&request_id, 504, "tunnel timeout")
            }
        }
    }

    /// Handle a WebSocket connection from a tunnel client.
    /// Runs for the lifetime of the connection.
    /// Called by the adapter after WS upgrade + bridge setup.
    pub async fn handle_connection(
        &self,
        mut incoming_rx: mpsc::Receiver<String>,
        outgoing_tx: mpsc::Sender<String>,
    ) {
        // Step 1: Wait for auth message
        let service = match self.handle_auth(&mut incoming_rx, &outgoing_tx).await {
            Some(service) => service,
            None => return,
        };

        // Step 2: Register service
        {
            let mut state = self.state.lock().await;
            state.services.insert(
                service.clone(),
                Connection {
                    outgoing_tx: outgoing_tx.clone(),
                },
            );
        }

        // Step 3: Listen for responses
        while let Some(raw) = incoming_rx.recv().await {
            let msg = match protocol::parse_message(&raw) {
                Ok(msg) => msg,
                Err(_) => continue,
            };

            if let Message::Response(response) = msg {
                let mut state = self.state.lock().await;
                if let Some(sender) = state.pending.remove(&response.request_id) {
                    let _ = sender.send(response);
                }
            }
        }

        // Step 4: Cleanup on disconnect
        self.cleanup_connection(&service).await;
    }

    /// Returns true if a client is connected for the given service.
    pub async fn is_connected(&self, service: &str) -> bool {
        let state = self.state.lock().await;
        state.services.contains_key(service)
    }

    async fn handle_auth(
        &self,
        incoming_rx: &mut mpsc::Receiver<String>,
        outgoing_tx: &mpsc::Sender<String>,
    ) -> Option<String> {
        let raw = incoming_rx.recv().await?;

        let msg = match protocol::parse_message(&raw) {
            Ok(Message::Auth(auth)) => auth,
            _ => {
                let err = protocol::AuthError {
                    msg_type: "auth_error".to_string(),
                    message: "expected auth message".to_string(),
                };
                let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
                return None;
            }
        };

        let valid = {
            let state = self.state.lock().await;
            constant_time_eq(&msg.token, &state.config.secret)
        };

        if !valid {
            let err = protocol::AuthError {
                msg_type: "auth_error".to_string(),
                message: "invalid token".to_string(),
            };
            let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
            return None;
        }

        let ok = protocol::AuthOk {
            msg_type: "auth_ok".to_string(),
        };
        let _ = outgoing_tx.send(serde_json::to_string(&ok).unwrap()).await;

        Some(msg.service)
    }

    async fn cleanup_connection(&self, service: &str) {
        let mut state = self.state.lock().await;
        state.services.remove(service);

        // Fail any pending requests that were waiting on this service's connection
        // We can't easily know which pending requests were for this service,
        // but the oneshot senders will fail naturally when the connection drops.
    }
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
