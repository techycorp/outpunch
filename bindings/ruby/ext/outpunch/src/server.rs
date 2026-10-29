use std::collections::HashMap;
use std::panic;
use std::sync::OnceLock;
use std::time::Duration;

use magnus::{Error, ExceptionClass, RHash, RModule, RString, Ruby, prelude::*, wrap};
use outpunch_core::protocol::{IncomingRequest, TunnelResponse};
use outpunch_core::server::{
    Connection as CoreConnection, OutpunchServer, ServerConfig, TunnelError,
};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};
use tokio::task::{JoinError, JoinHandle};

use crate::gvl::without_gvl;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("outpunch")
            .build()
            .expect("failed to start outpunch runtime")
    })
}

/// Wait for a task on the runtime with the GVL released. An interrupt that raises nothing
/// (Thread#wakeup, the VM timer) keeps waiting; one that raises returns `Err` and leaves the
/// task running for the caller to stop.
fn wait_for<T>(ruby: &Ruby, task: &mut JoinHandle<T>) -> Result<T, Error> {
    loop {
        let joined = without_gvl(ruby, |cancel| {
            runtime().block_on(async {
                tokio::select! {
                    joined = &mut *task => Some(joined),
                    _ = cancel.notified() => None,
                }
            })
        })?;
        if let Some(joined) = joined {
            return joined.map_err(|err| task_failed(ruby, err));
        }
    }
}

fn task_failed(ruby: &Ruby, err: JoinError) -> Error {
    if err.is_panic() {
        panic::resume_unwind(err.into_panic());
    }
    Error::new(
        ruby.exception_runtime_error(),
        format!("outpunch task failed: {err}"),
    )
}

fn outpunch_error(ruby: &Ruby, name: &str) -> Result<ExceptionClass, Error> {
    ruby.class_object()
        .const_get::<_, RModule>("Outpunch")?
        .const_get(name)
}

#[wrap(class = "Outpunch::Server", free_immediately, size)]
pub struct Server {
    inner: OutpunchServer,
}

impl Server {
    fn new(secret: String, timeout: f64, max_body_size: Option<usize>) -> Self {
        let defaults = ServerConfig::default();
        Self {
            inner: OutpunchServer::new(ServerConfig {
                secret,
                timeout: Duration::from_secs_f64(timeout),
                max_body_size: max_body_size.unwrap_or(defaults.max_body_size),
                ..defaults
            }),
        }
    }

    fn create_connection(&self) -> Connection {
        let inner = self.inner.create_connection();
        let (tx, rx) = mpsc::unbounded_channel();
        inner.on_message(move |msg| {
            let _ = tx.send(msg);
        });
        Connection {
            inner,
            outgoing: Mutex::new(rx),
        }
    }

    fn is_connected(ruby: &Ruby, rb_self: &Self, service: String) -> Result<bool, Error> {
        loop {
            let result = without_gvl(ruby, |cancel| {
                runtime().block_on(async {
                    tokio::select! {
                        connected = rb_self.inner.is_connected(&service) => Some(connected),
                        _ = cancel.notified() => None,
                    }
                })
            })?;
            if let Some(connected) = result {
                return Ok(connected);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_request(
        ruby: &Ruby,
        rb_self: &Self,
        service: String,
        method: String,
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
        body: Option<RString>,
    ) -> Result<RHash, Error> {
        // SAFETY: the slice is copied before the GVL is released.
        let body = body
            .map(|b| String::from_utf8_lossy(unsafe { b.as_slice() }).into_owned())
            .filter(|b| !b.is_empty());
        let incoming = IncomingRequest {
            service: service.clone(),
            method,
            path,
            query,
            headers,
            body,
        };

        let server = rb_self.inner.clone();
        let mut task = runtime().spawn(async move { server.try_handle_request(incoming).await });
        let result = wait_for(ruby, &mut task).inspect_err(|_| task.abort())?;

        match result {
            Ok(response) => response_hash(ruby, response),
            Err(TunnelError::NotConnected) => Err(Error::new(
                outpunch_error(ruby, "NotConnectedError")?,
                format!("Service '{service}' not connected"),
            )),
            Err(TunnelError::Timeout) => Err(Error::new(
                outpunch_error(ruby, "TunnelTimeoutError")?,
                format!("Tunnel timeout for service '{service}'"),
            )),
            Err(err) => Err(Error::new(
                outpunch_error(ruby, "TunnelError")?,
                err.to_string(),
            )),
        }
    }
}

fn response_hash(ruby: &Ruby, response: TunnelResponse) -> Result<RHash, Error> {
    let hash = ruby.hash_new();
    hash.aset("request_id", response.request_id)?;
    hash.aset("status", response.status)?;
    hash.aset("headers", response.headers)?;
    hash.aset("body", response.body)?;
    hash.aset("body_encoding", response.body_encoding)?;
    Ok(hash)
}

#[wrap(class = "Outpunch::Connection", free_immediately, size)]
pub struct Connection {
    inner: CoreConnection,
    outgoing: Mutex<mpsc::UnboundedReceiver<String>>,
}

impl Connection {
    /// Feed one WebSocket text message from the tunnel client into the core. Blocks while the
    /// core's queue is full.
    fn push_message(ruby: &Ruby, rb_self: &Self, text: String) -> Result<(), Error> {
        // A cancelled send delivered nothing, so send again until it lands or Ruby raises.
        loop {
            let sent = without_gvl(ruby, |cancel| {
                runtime().block_on(async {
                    tokio::select! {
                        _ = rb_self.inner.push_message(text.clone()) => Some(()),
                        _ = cancel.notified() => None,
                    }
                })
            })?;
            if sent.is_some() {
                return Ok(());
            }
        }
    }

    /// Block until the core has a message for the tunnel client. Returns nil once the
    /// connection has ended.
    fn next_outgoing(ruby: &Ruby, rb_self: &Self) -> Result<Option<String>, Error> {
        loop {
            let result = without_gvl(ruby, |cancel| {
                runtime().block_on(async {
                    let mut outgoing = rb_self.outgoing.lock().await;
                    tokio::select! {
                        msg = outgoing.recv() => Some(msg),
                        _ = cancel.notified() => None,
                    }
                })
            })?;
            if let Some(msg) = result {
                return Ok(msg);
            }
        }
    }

    fn close(&self) {
        self.inner.close();
    }

    /// Run the connection lifecycle (auth, relay, cleanup). Blocks until the connection ends.
    /// An exception raised into this thread (Thread#raise, Thread#kill, Timeout) closes the
    /// connection and then propagates.
    fn run(ruby: &Ruby, rb_self: &Self) -> Result<(), Error> {
        let inner = rb_self.inner.clone();
        let mut task = runtime().spawn(async move { inner.run().await });
        wait_for(ruby, &mut task).inspect_err(|_| {
            rb_self.inner.close();
            let _ = runtime().block_on(task);
        })
    }
}

pub fn init(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    ruby.require("timeout")?;
    let timeout_error: ExceptionClass = ruby
        .class_object()
        .const_get::<_, RModule>("Timeout")?
        .const_get("Error")?;
    module.define_error("NotConnectedError", ruby.exception_runtime_error())?;
    module.define_error("TunnelError", ruby.exception_runtime_error())?;
    module.define_error("TunnelTimeoutError", timeout_error)?;

    let server = module.define_class("Server", ruby.class_object())?;
    server.define_singleton_method("new", magnus::function!(Server::new, 3))?;
    server.define_method(
        "create_connection",
        magnus::method!(Server::create_connection, 0),
    )?;
    server.define_method("connected?", magnus::method!(Server::is_connected, 1))?;
    server.define_method("handle_request", magnus::method!(Server::handle_request, 6))?;

    let connection = module.define_class("Connection", ruby.class_object())?;
    connection.define_method("push_message", magnus::method!(Connection::push_message, 1))?;
    connection.define_method(
        "next_outgoing",
        magnus::method!(Connection::next_outgoing, 0),
    )?;
    connection.define_method("close", magnus::method!(Connection::close, 0))?;
    connection.define_method("run", magnus::method!(Connection::run, 0))?;

    Ok(())
}
