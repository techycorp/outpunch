use std::sync::Arc;

use rustls::ClientConfig;
use rustls::crypto::aws_lc_rs;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio_tungstenite::Connector;

/// Build the TLS connector for the tunnel WebSocket.
///
/// The provider is passed explicitly because this crate runs inside host processes (Python, Node,
/// Ruby) where the process-wide rustls default belongs to no one in particular.
pub(crate) fn connector() -> Result<Connector, rustls::Error> {
    let config = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}
