use std::sync::Mutex;
use std::time::Duration;

use magnus::{function, method, prelude::*, wrap, Error, Ruby};

struct ClientConfigInner {
    server_url: String,
    secret: String,
    service: String,
    forward_to: String,
    reconnect_delay: f64,
    request_timeout: f64,
}

#[wrap(class = "Outpunch::ClientConfig", free_immediately, size)]
pub struct ClientConfig(Mutex<ClientConfigInner>);

impl ClientConfig {
    fn new(
        server_url: String,
        secret: String,
        service: String,
        forward_to: Option<String>,
        reconnect_delay: Option<f64>,
        request_timeout: Option<f64>,
    ) -> Self {
        Self(Mutex::new(ClientConfigInner {
            server_url,
            secret,
            service,
            forward_to: forward_to.unwrap_or_else(|| "http://localhost:8080".into()),
            reconnect_delay: reconnect_delay.unwrap_or(5.0),
            request_timeout: request_timeout.unwrap_or(25.0),
        }))
    }

    fn to_rust(&self) -> outpunch_client::ClientConfig {
        let inner = self.0.lock().unwrap();
        outpunch_client::ClientConfig {
            server_url: inner.server_url.clone(),
            secret: inner.secret.clone(),
            service: inner.service.clone(),
            forward_to: inner.forward_to.clone(),
            reconnect_delay: Duration::from_secs_f64(inner.reconnect_delay),
            request_timeout: Duration::from_secs_f64(inner.request_timeout),
        }
    }

    fn server_url(&self) -> String {
        self.0.lock().unwrap().server_url.clone()
    }

    fn set_server_url(&self, v: String) {
        self.0.lock().unwrap().server_url = v;
    }

    fn secret(&self) -> String {
        self.0.lock().unwrap().secret.clone()
    }

    fn set_secret(&self, v: String) {
        self.0.lock().unwrap().secret = v;
    }

    fn service(&self) -> String {
        self.0.lock().unwrap().service.clone()
    }

    fn set_service(&self, v: String) {
        self.0.lock().unwrap().service = v;
    }

    fn forward_to(&self) -> String {
        self.0.lock().unwrap().forward_to.clone()
    }

    fn set_forward_to(&self, v: String) {
        self.0.lock().unwrap().forward_to = v;
    }

    fn reconnect_delay(&self) -> f64 {
        self.0.lock().unwrap().reconnect_delay
    }

    fn set_reconnect_delay(&self, v: f64) {
        self.0.lock().unwrap().reconnect_delay = v;
    }

    fn request_timeout(&self) -> f64 {
        self.0.lock().unwrap().request_timeout
    }

    fn set_request_timeout(&self, v: f64) {
        self.0.lock().unwrap().request_timeout = v;
    }

    fn inspect(&self) -> String {
        let inner = self.0.lock().unwrap();
        format!(
            "#<Outpunch::ClientConfig server_url={:?} service={:?} forward_to={:?}>",
            inner.server_url, inner.service, inner.forward_to
        )
    }
}

fn run(ruby: &Ruby, config: &ClientConfig) -> Result<(), Error> {
    let rust_config = config.to_rust();
    ruby.thread_call_without_gvl(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(outpunch_client::run(&rust_config));
    });
    Ok(())
}

fn run_connection(ruby: &Ruby, config: &ClientConfig) -> Result<(), Error> {
    let rust_config = config.to_rust();
    let result = ruby.thread_call_without_gvl(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(outpunch_client::run_connection(&rust_config))
    });
    result.map_err(|e| Error::new(ruby.exception_runtime_error(), e.to_string()))
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Outpunch")?;

    let config_class = module.define_class("ClientConfig", ruby.class_object())?;
    config_class.define_singleton_method("new", function!(ClientConfig::new, 6))?;
    config_class.define_method("server_url", method!(ClientConfig::server_url, 0))?;
    config_class.define_method("server_url=", method!(ClientConfig::set_server_url, 1))?;
    config_class.define_method("secret", method!(ClientConfig::secret, 0))?;
    config_class.define_method("secret=", method!(ClientConfig::set_secret, 1))?;
    config_class.define_method("service", method!(ClientConfig::service, 0))?;
    config_class.define_method("service=", method!(ClientConfig::set_service, 1))?;
    config_class.define_method("forward_to", method!(ClientConfig::forward_to, 0))?;
    config_class.define_method("forward_to=", method!(ClientConfig::set_forward_to, 1))?;
    config_class.define_method("reconnect_delay", method!(ClientConfig::reconnect_delay, 0))?;
    config_class.define_method(
        "reconnect_delay=",
        method!(ClientConfig::set_reconnect_delay, 1),
    )?;
    config_class.define_method("request_timeout", method!(ClientConfig::request_timeout, 0))?;
    config_class.define_method(
        "request_timeout=",
        method!(ClientConfig::set_request_timeout, 1),
    )?;
    config_class.define_method("inspect", method!(ClientConfig::inspect, 0))?;
    config_class.define_method("to_s", method!(ClientConfig::inspect, 0))?;

    module.define_module_function("run", function!(run, 2))?;
    module.define_module_function("run_connection", function!(run_connection, 2))?;

    Ok(())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
