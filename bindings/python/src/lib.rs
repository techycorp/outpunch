use std::time::Duration;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

#[pyclass(from_py_object)]
#[derive(Clone)]
struct ClientConfig {
    #[pyo3(get, set)]
    server_url: String,
    #[pyo3(get, set)]
    secret: String,
    #[pyo3(get, set)]
    service: String,
    #[pyo3(get, set)]
    forward_to: String,
    #[pyo3(get, set)]
    reconnect_delay: f64,
    #[pyo3(get, set)]
    request_timeout: f64,
}

#[pymethods]
impl ClientConfig {
    #[new]
    #[pyo3(signature = (
        server_url,
        secret,
        service,
        forward_to = "http://localhost:8080".to_string(),
        reconnect_delay = 5.0,
        request_timeout = 25.0,
    ))]
    fn new(
        server_url: String,
        secret: String,
        service: String,
        forward_to: String,
        reconnect_delay: f64,
        request_timeout: f64,
    ) -> Self {
        Self {
            server_url,
            secret,
            service,
            forward_to,
            reconnect_delay,
            request_timeout,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ClientConfig(server_url={:?}, service={:?}, forward_to={:?})",
            self.server_url, self.service, self.forward_to,
        )
    }
}

impl ClientConfig {
    fn to_rust(&self) -> outpunch_client::ClientConfig {
        outpunch_client::ClientConfig {
            server_url: self.server_url.clone(),
            secret: self.secret.clone(),
            service: self.service.clone(),
            forward_to: self.forward_to.clone(),
            reconnect_delay: Duration::from_secs_f64(self.reconnect_delay),
            request_timeout: Duration::from_secs_f64(self.request_timeout),
        }
    }
}

/// Run the tunnel client forever, reconnecting on failure.
/// This is a blocking call that releases the GIL.
#[pyfunction]
fn run(py: Python<'_>, config: &ClientConfig) -> PyResult<()> {
    let rust_config = config.to_rust();
    py.detach(|| {
        pyo3_async_runtimes::tokio::get_runtime().block_on(outpunch_client::run(&rust_config));
    });
    Ok(())
}

/// Run a single connection attempt. Returns a Python awaitable.
#[pyfunction]
fn run_connection<'py>(py: Python<'py>, config: &ClientConfig) -> PyResult<Bound<'py, PyAny>> {
    let rust_config = config.to_rust();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        outpunch_client::run_connection(&rust_config)
            .await
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(())
    })
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ClientConfig>()?;
    m.add_function(wrap_pyfunction!(run, m)?)?;
    m.add_function(wrap_pyfunction!(run_connection, m)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
