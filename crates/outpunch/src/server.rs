use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, oneshot};

use crate::protocol::{self, IncomingRequest, Message, TunnelRequest, TunnelResponse};

/// Why a tunnel request failed before the client produced a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelError {
    NotConnected,
    ConnectionLost,
    Disconnected,
    Timeout,
}

impl TunnelError {
    pub fn status(&self) -> u16 {
        match self {
            TunnelError::Timeout => 504,
            _ => 502,
        }
    }
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TunnelError::NotConnected => "no client connected for service",
            TunnelError::ConnectionLost => "client connection lost",
            TunnelError::Disconnected => "client disconnected",
            TunnelError::Timeout => "tunnel timeout",
        })
    }
}

impl std::error::Error for TunnelError {}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub secret: String,
    pub timeout: Duration,
    pub max_body_size: usize,
    /// How long a new connection may take to send its auth message.
    pub auth_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            timeout: Duration::from_secs(25),
            max_body_size: 10 * 1024 * 1024, // 10MB
            auth_timeout: Duration::from_secs(10),
        }
    }
}

/// Internal handle stored in the services map for sending messages to a client.
struct ServiceHandle {
    conn_id: u64,
    outgoing_tx: mpsc::Sender<String>,
    connection: Weak<ConnectionInner>,
}

/// A request waiting for a response from the connection it was sent to.
struct PendingRequest {
    conn_id: u64,
    tx: oneshot::Sender<TunnelResponse>,
}

/// Shared state for the outpunch server.
struct ServerState {
    config: ServerConfig,
    /// service name → active connection handle
    services: HashMap<String, ServiceHandle>,
    /// request_id → the request waiting on a response
    pending: HashMap<String, PendingRequest>,
}

#[derive(Clone)]
pub struct OutpunchServer {
    state: Arc<Mutex<ServerState>>,
    max_body_size: usize,
    next_conn_id: Arc<AtomicU64>,
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
            next_conn_id: Arc::new(AtomicU64::new(1)),
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
                id: self.next_conn_id.fetch_add(1, Ordering::Relaxed),
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

    /// Handle an incoming HTTP tunnel request. Tunnel failures become 502/504 error responses.
    pub async fn handle_request(&self, incoming: IncomingRequest) -> TunnelResponse {
        let tunnel_req = protocol::build_tunnel_request(&incoming);
        let request_id = tunnel_req.request_id.clone();
        match self.dispatch(tunnel_req).await {
            Ok(response) => response,
            Err(err) => protocol::error_response(&request_id, err.status(), &err.to_string()),
        }
    }

    /// Handle an incoming HTTP tunnel request, returning tunnel failures as a `TunnelError`.
    pub async fn try_handle_request(
        &self,
        incoming: IncomingRequest,
    ) -> Result<TunnelResponse, TunnelError> {
        self.dispatch(protocol::build_tunnel_request(&incoming))
            .await
    }

    async fn dispatch(&self, tunnel_req: TunnelRequest) -> Result<TunnelResponse, TunnelError> {
        let request_id = tunnel_req.request_id.clone();
        let json = protocol::serialize_request(&tunnel_req);

        let (resp_tx, resp_rx) = oneshot::channel();

        // Register the pending request before sending so a fast response can't be missed,
        // and send outside the lock so one slow client can't stall the whole server.
        let (outgoing_tx, timeout) = {
            let mut state = self.state.lock().await;
            let Some(conn) = state.services.get(&tunnel_req.service) else {
                return Err(TunnelError::NotConnected);
            };
            let conn_id = conn.conn_id;
            let outgoing_tx = conn.outgoing_tx.clone();
            state.pending.insert(
                request_id.clone(),
                PendingRequest {
                    conn_id,
                    tx: resp_tx,
                },
            );
            (outgoing_tx, state.config.timeout)
        };

        let exchange = async {
            outgoing_tx
                .send(json)
                .await
                .map_err(|_| TunnelError::ConnectionLost)?;
            resp_rx.await.map_err(|_| TunnelError::Disconnected)
        };

        let result = tokio::time::timeout(timeout, exchange)
            .await
            .unwrap_or(Err(TunnelError::Timeout));
        if result.is_err() {
            self.state.lock().await.pending.remove(&request_id);
        }
        result
    }

    /// Returns true if a client is connected for the given service.
    pub async fn is_connected(&self, service: &str) -> bool {
        let state = self.state.lock().await;
        state.services.contains_key(service)
    }

    /// Unregister a connection and fail the requests still waiting on it. A newer
    /// connection registered under the same service is left in place.
    async fn cleanup_connection(&self, service: &str, conn_id: u64) {
        let mut state = self.state.lock().await;
        if state
            .services
            .get(service)
            .is_some_and(|handle| handle.conn_id == conn_id)
        {
            state.services.remove(service);
        }
        state
            .pending
            .retain(|_, pending| pending.conn_id != conn_id);
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
    id: u64,
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

        // Register service. The newest connection wins; a replaced connection is closed.
        let replaced = {
            let mut state = self.inner.server.state.lock().await;
            state.services.insert(
                service.clone(),
                ServiceHandle {
                    conn_id: self.inner.id,
                    outgoing_tx: outgoing_tx.clone(),
                    connection: Arc::downgrade(&self.inner),
                },
            )
        };
        if let Some(old) = replaced.and_then(|handle| handle.connection.upgrade()) {
            old.incoming_tx.lock().unwrap().take();
        }

        // Relay loop: read responses from the client
        while let Some(raw) = incoming_rx.recv().await {
            let msg = match protocol::parse_message(&raw) {
                Ok(msg) => msg,
                Err(_) => continue,
            };

            if let Message::Response(response) = msg {
                let mut state = self.inner.server.state.lock().await;
                let ours = state
                    .pending
                    .get(&response.request_id)
                    .is_some_and(|pending| pending.conn_id == self.inner.id);
                if ours && let Some(pending) = state.pending.remove(&response.request_id) {
                    let _ = pending.tx.send(response);
                }
            }
        }

        // Cleanup
        self.inner
            .server
            .cleanup_connection(&service, self.inner.id)
            .await;
        drop(outgoing_tx);
        let _ = drain_handle.await;
    }

    async fn handle_auth(
        &self,
        incoming_rx: &mut mpsc::Receiver<String>,
        outgoing_tx: &mpsc::Sender<String>,
    ) -> Option<String> {
        let auth_timeout = self.inner.server.state.lock().await.config.auth_timeout;
        let Ok(raw) = tokio::time::timeout(auth_timeout, incoming_rx.recv()).await else {
            let err = protocol::AuthError {
                msg_type: "auth_error".to_string(),
                message: "auth timeout".to_string(),
            };
            let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
            return None;
        };
        let raw = raw?;

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
            !state.config.secret.is_empty() && constant_time_eq(&msg.token, &state.config.secret)
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
