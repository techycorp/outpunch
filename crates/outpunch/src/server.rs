use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

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
    /// How long an authenticated client may stay silent (no message, no WebSocket ping)
    /// before it is treated as gone: unregistered, its pending requests failed. Must be
    /// longer than the client's ping interval (30 s by default).
    pub client_idle_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            timeout: Duration::from_secs(25),
            max_body_size: 10 * 1024 * 1024, // 10MB
            auth_timeout: Duration::from_secs(10),
            client_idle_timeout: Duration::from_secs(90),
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
    timeout: Duration,
    next_conn_id: Arc<AtomicU64>,
}

impl OutpunchServer {
    pub fn new(config: ServerConfig) -> Self {
        trace_event!({"ev": "server", "secret_set": !config.secret.is_empty()});
        let max_body_size = config.max_body_size;
        let timeout = config.timeout;
        Self {
            state: Arc::new(Mutex::new(ServerState {
                config,
                services: HashMap::new(),
                pending: HashMap::new(),
            })),
            max_body_size,
            timeout,
            next_conn_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn max_body_size(&self) -> usize {
        self.max_body_size
    }

    /// The request timeout. Adapters also use it as the limit on one socket write, and close
    /// the connection when a write takes longer: a client that cannot take one message within
    /// a caller's whole time budget is not serving anyone.
    pub fn request_timeout(&self) -> Duration {
        self.timeout
    }

    /// Create a new connection for a tunnel client.
    pub fn create_connection(&self) -> Connection {
        let (incoming_tx, incoming_rx) = mpsc::channel::<String>(64);
        let (outgoing_tx, outgoing_rx) = mpsc::channel::<String>(64);
        let id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        trace_event!({"ev": "create", "conn": id});

        Connection {
            inner: Arc::new(ConnectionInner {
                id,
                server: self.clone(),
                incoming_tx: std::sync::Mutex::new(Some(incoming_tx)),
                last_heard: std::sync::Mutex::new(tokio::time::Instant::now()),
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
            let mut state = self.state.lock().unwrap();
            let Some(conn) = state.services.get(&tunnel_req.service) else {
                trace_event!({"ev": "start", "req": request_id, "svc": tunnel_req.service, "conn": null});
                return Err(TunnelError::NotConnected);
            };
            let conn_id = conn.conn_id;
            trace_event!({"ev": "start", "req": request_id, "svc": tunnel_req.service, "conn": conn_id});
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
        #[cfg_attr(not(feature = "trace"), allow(unused_mut))]
        let mut pending = PendingGuard {
            state: &self.state,
            request_id: &request_id,
            #[cfg(feature = "trace")]
            returned: false,
        };

        // Without the trace feature the trace_event! calls vanish, which leaves shapes clippy flags.
        #[cfg_attr(not(feature = "trace"), allow(clippy::let_and_return))]
        let exchange = async {
            outgoing_tx.send(json).await.map_err(|_| {
                trace_event!({"ev": "send_fail", "req": request_id});
                TunnelError::ConnectionLost
            })?;
            trace_event!({"ev": "send", "req": request_id});
            let response = resp_rx.await.map_err(|_| TunnelError::Disconnected);
            trace_event!({"ev": "receive", "req": request_id, "ok": response.is_ok()});
            response
        };

        #[cfg_attr(not(feature = "trace"), allow(clippy::unnecessary_lazy_evaluations))]
        let result = tokio::time::timeout(timeout, exchange)
            .await
            .unwrap_or_else(|_| {
                trace_event!({"ev": "timeout", "req": request_id});
                Err(TunnelError::Timeout)
            });
        #[cfg(feature = "trace")]
        {
            pending.returned = true;
            if result.is_err() {
                trace_event!({"ev": "remove", "req": request_id});
            }
        }
        drop(pending);
        result
    }

    /// False only for a `request` message whose request is no longer pending, i.e. its
    /// caller has already been answered (response, timeout, cancellation or disconnect).
    /// Every other message is pending. Adapters call this before writing an outgoing
    /// message to the socket, and drop the message when it returns false.
    pub fn is_pending(&self, msg: &str) -> bool {
        let Some(request_id) = protocol::request_id_of(msg) else {
            return true;
        };
        self.state.lock().unwrap().pending.contains_key(&request_id)
    }

    /// Returns true if a client is connected for the given service.
    pub async fn is_connected(&self, service: &str) -> bool {
        let state = self.state.lock().unwrap();
        state.services.contains_key(service)
    }

    /// Unregister a connection and fail the requests still waiting on it. A newer
    /// connection registered under the same service is left in place.
    fn cleanup_connection(&self, service: &str, conn_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ours = state
            .services
            .get(service)
            .is_some_and(|handle| handle.conn_id == conn_id);
        trace_event!({
            "ev": "cleanup",
            "conn": conn_id,
            "unregistered": ours,
            "failed": state
                .pending
                .iter()
                .filter(|(_, pending)| pending.conn_id == conn_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        });
        if ours {
            state.services.remove(service);
        }
        state
            .pending
            .retain(|_, pending| pending.conn_id != conn_id);
    }
}

/// Removes a request's pending entry when its dispatch ends, on every path: a response,
/// an error, or the dispatch future being dropped before it completes.
struct PendingGuard<'a> {
    state: &'a Mutex<ServerState>,
    request_id: &'a str,
    /// dispatch() returned (as opposed to its future being dropped).
    #[cfg(feature = "trace")]
    returned: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(feature = "trace")]
        if !self.returned {
            trace_event!({"ev": "guard_drop", "req": self.request_id});
        }
        state.pending.remove(self.request_id);
    }
}

/// Runs `cleanup_connection` when `run()` ends after registering, on every path: the relay
/// loop ending, or the `run()` future being dropped (task abort, `select!`, runtime shutdown).
struct CleanupGuard<'a> {
    server: &'a OutpunchServer,
    service: &'a str,
    conn_id: u64,
}

impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) {
        self.server.cleanup_connection(self.service, self.conn_id);
    }
}

/// Closes the connection when dropped. The drain task holds one, so the connection closes
/// on every way the task can end, including a panic in the callback.
struct CloseOnDrop(Weak<ConnectionInner>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let Some(inner) = self.0.upgrade() {
            if std::thread::panicking() {
                trace_event!({"ev": "drain_death", "conn": inner.id});
            }
            Connection { inner }.close();
        }
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
    /// When the client was last heard from: a message or a WebSocket ping.
    last_heard: std::sync::Mutex<tokio::time::Instant>,
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

/// A handle to a `Connection` that does not keep it alive. Useful inside the `on_message`
/// callback, which the connection itself owns.
#[derive(Clone)]
pub struct WeakConnection {
    inner: Weak<ConnectionInner>,
}

impl WeakConnection {
    pub fn upgrade(&self) -> Option<Connection> {
        self.inner.upgrade().map(|inner| Connection { inner })
    }
}

impl Connection {
    pub fn downgrade(&self) -> WeakConnection {
        WeakConnection {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Push an incoming WebSocket message into the connection.
    pub async fn push_message(&self, text: String) {
        self.heartbeat();
        let tx = {
            let guard = self.inner.incoming_tx.lock().unwrap();
            guard.as_ref().cloned()
        };
        if let Some(tx) = tx {
            #[cfg(feature = "trace")]
            let traced = {
                let secret = self
                    .inner
                    .server
                    .state
                    .lock()
                    .unwrap()
                    .config
                    .secret
                    .clone();
                crate::trace::message(&text, &secret)
            };
            if tx.send(text).await.is_ok() {
                trace_event!({"ev": "push", "conn": self.inner.id, "msg": traced});
            }
        }
    }

    /// Record that the client is alive. Adapters call it for every WebSocket ping frame;
    /// `push_message` calls it for every message.
    pub fn heartbeat(&self) {
        *self
            .inner
            .last_heard
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = tokio::time::Instant::now();
    }

    /// Register a callback for outgoing messages (messages to send on the WebSocket).
    /// Must be called before `run()`.
    pub fn on_message(&self, callback: impl Fn(String) + Send + Sync + 'static) {
        *self.inner.callback.lock().unwrap() = Some(Box::new(callback));
    }

    /// Signal the connection to close. Causes `run()` to exit.
    pub fn close(&self) {
        #[cfg_attr(not(feature = "trace"), allow(unused_variables))]
        let closed = self
            .inner
            .incoming_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .is_some();
        if closed {
            trace_event!({"ev": "close", "conn": self.inner.id});
        }
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
        trace_event!({"ev": "run", "conn": self.inner.id});

        let callback = self.inner.callback.lock().unwrap().take();

        let ConnectionRunState {
            mut incoming_rx,
            mut outgoing_rx,
            outgoing_tx,
        } = run_state;

        // Spawn outgoing drain task: delivers messages via callback. If it ends early (the
        // callback panicked), the connection closes so run() cleans up.
        let close_on_end = CloseOnDrop(Arc::downgrade(&self.inner));
        let drain_handle = tokio::spawn(async move {
            let _close_on_end = close_on_end;
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
        // auth_ok is queued under the same lock, so a client that has it is already routable,
        // and it is the first message the client gets: no request can be queued before it.
        // The channel is still empty here, so try_send cannot fail for lack of room.
        #[cfg_attr(not(feature = "trace"), allow(clippy::let_and_return))]
        let replaced = {
            let mut state = self.inner.server.state.lock().unwrap();
            let replaced = state.services.insert(
                service.clone(),
                ServiceHandle {
                    conn_id: self.inner.id,
                    outgoing_tx: outgoing_tx.clone(),
                    connection: Arc::downgrade(&self.inner),
                },
            );
            let ok = protocol::AuthOk {
                msg_type: "auth_ok".to_string(),
            };
            let _ = outgoing_tx.try_send(serde_json::to_string(&ok).unwrap());
            trace_event!({
                "ev": "register",
                "conn": self.inner.id,
                "old": replaced.as_ref().map(|handle| handle.conn_id),
            });
            replaced
        };
        let cleanup = CleanupGuard {
            server: &self.inner.server,
            service: &service,
            conn_id: self.inner.id,
        };
        if let Some(old) = replaced.and_then(|handle| handle.connection.upgrade()) {
            old.incoming_tx.lock().unwrap().take();
        }
        trace_event!({"ev": "kick", "conn": self.inner.id});

        // Relay loop: read responses from the client, until the connection closes or the
        // client has been silent for client_idle_timeout.
        let idle_timeout = self
            .inner
            .server
            .state
            .lock()
            .unwrap()
            .config
            .client_idle_timeout;
        loop {
            let deadline = *self.inner.last_heard.lock().unwrap() + idle_timeout;
            tokio::select! {
                raw = incoming_rx.recv() => {
                    let Some(raw) = raw else {
                        trace_event!({"ev": "relay_end", "conn": self.inner.id});
                        break;
                    };
                    #[cfg_attr(not(feature = "trace"), allow(unused_variables))]
                    let completed = self.deliver_response(&raw);
                    trace_event!({"ev": "relay", "conn": self.inner.id, "req": completed});
                }
                _ = tokio::time::sleep_until(deadline) => {
                    if self.inner.last_heard.lock().unwrap().elapsed() >= idle_timeout {
                        trace_event!({"ev": "idle", "conn": self.inner.id});
                        break;
                    }
                }
            }
        }

        drop(cleanup);
        drop(outgoing_tx);
        let _ = drain_handle.await;
    }

    /// Hand a client's response to the request waiting for it, if that request was sent to
    /// this connection. Returns the id of the request it completed.
    fn deliver_response(&self, raw: &str) -> Option<String> {
        let Ok(Message::Response(response)) = protocol::parse_message(raw) else {
            return None;
        };
        let mut state = self.inner.server.state.lock().unwrap();
        let ours = state
            .pending
            .get(&response.request_id)
            .is_some_and(|pending| pending.conn_id == self.inner.id);
        if !ours {
            return None;
        }
        let pending = state.pending.remove(&response.request_id)?;
        let request_id = response.request_id.clone();
        let _ = pending.tx.send(response);
        Some(request_id)
    }

    async fn handle_auth(
        &self,
        incoming_rx: &mut mpsc::Receiver<String>,
        outgoing_tx: &mpsc::Sender<String>,
    ) -> Option<String> {
        let auth_timeout = self.inner.server.state.lock().unwrap().config.auth_timeout;
        let Ok(raw) = tokio::time::timeout(auth_timeout, incoming_rx.recv()).await else {
            trace_event!({"ev": "auth_end", "conn": self.inner.id});
            let err = protocol::AuthError {
                msg_type: "auth_error".to_string(),
                message: "auth timeout".to_string(),
            };
            let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
            return None;
        };
        #[cfg_attr(not(feature = "trace"), allow(clippy::question_mark))]
        let Some(raw) = raw else {
            trace_event!({"ev": "auth_end", "conn": self.inner.id});
            return None;
        };

        let msg = match protocol::parse_message(&raw) {
            Ok(Message::Auth(auth)) => auth,
            _ => {
                trace_event!({"ev": "auth", "conn": self.inner.id, "ok": false});
                let err = protocol::AuthError {
                    msg_type: "auth_error".to_string(),
                    message: "expected auth message".to_string(),
                };
                let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
                return None;
            }
        };

        let valid = {
            let state = self.inner.server.state.lock().unwrap();
            !state.config.secret.is_empty() && constant_time_eq(&msg.token, &state.config.secret)
        };

        trace_event!({"ev": "auth", "conn": self.inner.id, "ok": valid});
        if !valid {
            let err = protocol::AuthError {
                msg_type: "auth_error".to_string(),
                message: "invalid token".to_string(),
            };
            let _ = outgoing_tx.send(serde_json::to_string(&err).unwrap()).await;
            return None;
        }

        Some(msg.service)
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
