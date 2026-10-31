#[allow(dead_code)]
#[path = "fakes/tls_server.rs"]
mod tls_server;

use outpunch_client::{ClientConfig, run_connection};

#[tokio::test]
async fn rustls_rejects_a_certificate_outside_the_trust_store() {
    let server = tls_server::spawn().await;
    let config = ClientConfig {
        server_url: server.url,
        ..ClientConfig::default()
    };

    let error = run_connection(&config).await.unwrap_err().to_string();

    assert!(
        error.contains("invalid peer certificate"),
        "expected a rustls certificate error, got: {error}"
    );
}
