use std::time::Duration;

use outpunch::server::{OutpunchServer, ServerConfig};

#[tokio::main]
async fn main() {
    let secret = std::env::var("OUTPUNCH_SECRET").unwrap_or_else(|_| "dev-secret".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());

    let server = OutpunchServer::new(ServerConfig {
        secret,
        timeout: Duration::from_secs(25),
        ..ServerConfig::default()
    });

    let app = outpunch_axum::router(server);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap();

    eprintln!("outpunch server listening on port {port}");
    axum::serve(listener, app).await.unwrap();
}
