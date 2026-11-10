use std::time::Duration;

use outpunch::server::{OutpunchServer, ServerConfig};

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(filter)
        .init();
    let secret = std::env::var("OUTPUNCH_SECRET").unwrap_or_else(|_| "dev-secret".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());

    let defaults = ServerConfig::default();
    let seconds = |name: &str, default: Duration| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .map_or(default, Duration::from_secs)
    };
    let server = OutpunchServer::new(ServerConfig {
        secret,
        timeout: seconds("OUTPUNCH_TIMEOUT", defaults.timeout),
        client_idle_timeout: seconds("OUTPUNCH_CLIENT_IDLE_TIMEOUT", defaults.client_idle_timeout),
        ..defaults
    });

    let app = outpunch_axum::router(server);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap();

    tracing::info!(port = %port, "listening");
    axum::serve(listener, app).await.unwrap();
}
