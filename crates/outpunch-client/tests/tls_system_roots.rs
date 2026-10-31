//! Linux only: the platform verifier reads `SSL_CERT_FILE` there, while macOS and Windows use the
//! OS trust store. Alone in its binary because it changes the process environment.
#![cfg(target_os = "linux")]

#[path = "fakes/tls_server.rs"]
mod tls_server;

use outpunch_client::{ClientConfig, run_connection};

#[tokio::test]
async fn client_trusts_a_private_ca_from_the_system_store() {
    let server = tls_server::spawn().await;
    let ca_file = std::env::temp_dir().join(format!("outpunch-ca-{}.pem", std::process::id()));
    std::fs::write(&ca_file, &server.ca_pem).unwrap();
    // SAFETY: this binary has one test, and the server task does not read the environment.
    unsafe { std::env::set_var("SSL_CERT_FILE", &ca_file) };
    let config = ClientConfig {
        server_url: server.url,
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    std::fs::remove_file(&ca_file).unwrap();

    assert!(result.is_ok(), "connection failed: {result:?}");
}
