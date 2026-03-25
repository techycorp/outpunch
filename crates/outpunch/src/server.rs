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

/// Internal handle stored in the services map for sending messages to a client.
struct ServiceHandle {
    outgoing_tx: mpsc::Sender<String>,
}

/// Shared state for the outpunch server.
struct ServerState {
    config: ServerConfig,
    /// service name → active connection handle
    services: HashMap<String, ServiceHandle>,
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

    /// Create a new connection for a tunnel client.
    pub fn create_connection(&self) -> Connection {
        let (incoming_tx, incoming_rx) = mpsc::channel::<String>(64);
        let (outgoing_tx, outgoing_rx) = mpsc::channel::<String>(64);

        Connection {
            inner: Arc::new(ConnectionInner {
                server: self.clone(),
                incoming_tx: std::sync::Mutex::new(Some(incoming_tx)),
                callback: std::sync::Mutex::new(None),
                run_state: std::sync::Mutex::new(Some(ConnectionRunState {
                    incoming_rx,
                    outgoing_rx,
                    outgoing_tx,
                })),
            }),
        }
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

    /// Returns true if a client is connected for the given service.
    pub async fn is_connected(&self, service: &str) -> bool {
        let state = self.state.lock().await;
        state.services.contains_key(service)
    }

    async fn cleanup_connection(&self, service: &str) {
        let mut state = self.state.lock().await;
        state.services.remove(service);
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

// --- Connection ---

struct ConnectionRunState {
    incoming_rx: mpsc::Receiver<String>,
    outgoing_rx: mpsc::Receiver<String>,
    outgoing_tx: mpsc::Sender<String>,
}

type MessageCallback = Box<dyn Fn(String) + Send + Sync>;

struct ConnectionInner {
    server: OutpunchServer,
    incoming_tx: std::sync::Mutex<Option<mpsc::Sender<String>>>,
    callback: std::sync::Mutex<Option<MessageCallback>>,
    run_state: std::sync::Mutex<Option<ConnectionRunState>>,
}

/// A tunnel client connection. Created by `OutpunchServer::create_connection()`.
///
/// The adapter pushes incoming WebSocket messages via `push_message()`,
/// registers a callback for outgoing messages via `on_message()`,
/// and drives the connection lifecycle via `run()`.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<ConnectionInner>,
}

impl Connection {
    /// Push an incoming WebSocket message into the connection.
    pub async fn push_message(&self, text: String) {
        let tx = {
            let guard = self.inner.incoming_tx.lock().unwrap();
            guard.as_ref().cloned()
        };
        if let Some(tx) = tx {
            let _ = tx.send(text).await;
        }
    }

    /// Register a callback for outgoing messages (messages to send on the WebSocket).
    /// Must be called before `run()`.
    pub fn on_message(&self, callback: impl Fn(String) + Send + Sync + 'static) {
        *self.inner.callback.lock().unwrap() = Some(Box::new(callback));
    }

    /// Signal the connection to close. Causes `run()` to exit.
    pub fn close(&self) {
        self.inner.incoming_tx.lock().unwrap().take();
    }

    /// Run the connection lifecycle: auth, request relay, cleanup.
    /// Blocks until the connection ends (via `close()` or channel drop).
    /// Can only be called once.
    pub async fn run(&self) {
        let run_state = self
            .inner
            .run_state
            .lock()
            .unwrap()
            .take()
            .expect("run() can only be called once");

        let callback = self.inner.callback.lock().unwrap().take();

        let ConnectionRunState {
            mut incoming_rx,
            mut outgoing_rx,
            outgoing_tx,
        } = run_state;

        // Spawn outgoing drain task: delivers messages via callback
        let drain_handle = tokio::spawn(async move {
            while let Some(msg) = outgoing_rx.recv().await {
                if let Some(ref cb) = callback {
                    cb(msg);
                }
            }
        });

        // Auth
        let service = match self.handle_auth(&mut incoming_rx, &outgoing_tx).await {
            Some(service) => service,
            None => {
                drop(outgoing_tx);
                let _ = drain_handle.await;
                return;
            }
        };

        // Register service
        {
            let mut state = self.inner.server.state.lock().await;
            state.services.insert(
                service.clone(),
                ServiceHandle {
                    outgoing_tx: outgoing_tx.clone(),
                },
            );
        }

        // Relay loop: read responses from the client
        while let Some(raw) = incoming_rx.recv().await {
            let msg = match protocol::parse_message(&raw) {
                Ok(msg) => msg,
                Err(_) => continue,
            };

            if let Message::Response(response) = msg {
                let mut state = self.inner.server.state.lock().await;
                if let Some(sender) = state.pending.remove(&response.request_id) {
                    let _ = sender.send(response);
                }
            }
        }

        // Cleanup
        self.inner.server.cleanup_connection(&service).await;
        drop(outgoing_tx);
        let _ = drain_handle.await;
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
            let state = self.inner.server.state.lock().await;
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
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
