#![deny(clippy::all)]

use std::collections::HashMap;
use std::time::Duration;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

#[napi(object)]
pub struct ServerConfig {
    pub secret: String,
    pub timeout_ms: Option<u32>,
    pub max_body_size: Option<u32>,
}

#[napi]
pub struct OutpunchServer {
    inner: outpunch::server::OutpunchServer,
}

#[napi]
impl OutpunchServer {
    #[napi(constructor)]
    pub fn new(config: ServerConfig) -> Self {
        let inner = outpunch::server::OutpunchServer::new(outpunch::server::ServerConfig {
            secret: config.secret,
            timeout: Duration::from_millis(config.timeout_ms.unwrap_or(25000) as u64),
            max_body_size: config.max_body_size.unwrap_or(10 * 1024 * 1024) as usize,
        });
        Self { inner }
    }

    #[napi]
    pub fn create_connection(&self) -> OutpunchConnection {
        OutpunchConnection {
            inner: self.inner.create_connection(),
        }
    }

    #[napi]
    pub async fn handle_request(&self, request: IncomingRequest) -> Result<TunnelResponse> {
        let incoming = outpunch::protocol::IncomingRequest {
            service: request.service,
            method: request.method,
            path: request.path,
            query: request.query.unwrap_or_default(),
            headers: request.headers.unwrap_or_default(),
            body: request.body,
        };
        let resp = self.inner.handle_request(incoming).await;
        Ok(TunnelResponse {
            status: resp.status,
            headers: resp.headers,
            body: resp.body,
            body_encoding: resp.body_encoding,
            request_id: resp.request_id,
        })
    }
}

#[napi(object)]
pub struct IncomingRequest {
    pub service: String,
    pub method: String,
    pub path: String,
    pub query: Option<HashMap<String, String>>,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
}

#[napi(object)]
pub struct TunnelResponse {
    pub request_id: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub body_encoding: Option<String>,
}

#[napi]
pub struct OutpunchConnection {
    inner: outpunch::server::Connection,
}

#[napi]
impl OutpunchConnection {
    #[napi]
    pub async fn push_message(&self, text: String) -> Result<()> {
        self.inner.push_message(text).await;
        Ok(())
    }

    #[napi(ts_args_type = "callback: (err: null, msg: string) => void")]
    pub fn on_message(&self, callback: ThreadsafeFunction<String>) -> Result<()> {
        self.inner.on_message(move |msg| {
            callback.call(Ok(msg), ThreadsafeFunctionCallMode::NonBlocking);
        });
        Ok(())
    }

    #[napi]
    pub fn close(&self) {
        self.inner.close();
    }

    #[napi]
    pub async fn run(&self) -> Result<()> {
        self.inner.run().await;
        Ok(())
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
