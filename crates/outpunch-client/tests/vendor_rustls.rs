//! Facts about real TLS endpoints, reached through the client's own connection path.
//! badssl.com serves one deliberate TLS condition per host and speaks no WebSocket, so a
//! successful TLS handshake surfaces as an HTTP-level upgrade failure.
//!
//! Run with: cargo test -p outpunch-client --test vendor_rustls -- --ignored

use outpunch_client::{ClientConfig, run_connection};

async fn connection_error(url: &str) -> String {
    let config = ClientConfig {
        server_url: url.to_string(),
        ..ClientConfig::default()
    };
    run_connection(&config).await.unwrap_err().to_string()
}

fn assert_tls_handshake_passed(error: &str) {
    assert!(
        error.starts_with("HTTP error"),
        "expected TLS to pass and the WebSocket upgrade to fail, got: {error}"
    );
}

fn assert_certificate_rejected(error: &str) {
    assert!(
        error.contains("invalid peer certificate"),
        "expected a certificate rejection, got: {error}"
    );
}

fn assert_tls_refused(error: &str) {
    assert!(
        error.contains("received fatal alert"),
        "expected the handshake to end in a TLS alert, got: {error}"
    );
}

/// Production wss:// connections depend on public CA chains validating through system roots.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_accept_a_public_ca_chain_through_system_roots() {
    assert_tls_handshake_passed(&connection_error("wss://sha256.badssl.com/").await);
}

/// The client must not tunnel traffic to a server with a lapsed certificate.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_reject_an_expired_certificate() {
    assert_certificate_rejected(&connection_error("wss://expired.badssl.com/").await);
}

/// The client must not tunnel traffic to a host the certificate does not name.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_reject_a_certificate_for_another_host() {
    assert_certificate_rejected(&connection_error("wss://wrong.host.badssl.com/").await);
}

/// tests/fakes/tls_server.rs relies on an unknown issuer failing as a certificate error.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_reject_an_untrusted_root() {
    assert_certificate_rejected(&connection_error("wss://untrusted-root.badssl.com/").await);
}

/// tests/fakes/tls_server.rs relies on a self-signed chain failing as a certificate error.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_reject_a_self_signed_certificate() {
    assert_certificate_rejected(&connection_error("wss://self-signed.badssl.com/").await);
}

/// Servers that have not moved to TLS 1.3 must stay reachable.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_accept_a_tls_1_2_only_server() {
    assert_tls_handshake_passed(&connection_error("wss://tls-v1-2.badssl.com:1012/").await);
}

/// AGENTS.md states TLS 1.0 endpoints are unsupported.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_refuse_a_tls_1_0_only_server() {
    assert_tls_refused(&connection_error("wss://tls-v1-0.badssl.com:1010/").await);
}

/// AGENTS.md states TLS 1.1 endpoints are unsupported.
#[tokio::test]
#[ignore = "vendor test: needs network access to badssl.com"]
async fn rustls_must_refuse_a_tls_1_1_only_server() {
    assert_tls_refused(&connection_error("wss://tls-v1-1.badssl.com:1011/").await);
}
