//! Local stand-in for a public outpunch server behind TLS termination. Its certificate chains to
//! a private CA generated per test, so no real trust store accepts it by default.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_tungstenite::tungstenite::Message;

pub struct TlsServer {
    pub url: String,
    pub ca_pem: String,
}

/// Serve one WebSocket connection at a time: read the auth message, answer `auth_ok`, close.
pub async fn spawn() -> TlsServer {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);

    let leaf_key = KeyPair::generate().unwrap();
    let leaf_cert = CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();

    let chain: Vec<CertificateDer<'static>> = vec![leaf_cert.der().clone()];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
    let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_protocol_versions(rustls::DEFAULT_VERSIONS)
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut ws = tokio_tungstenite::accept_async(tls).await.unwrap();
                ws.next().await;
                ws.send(Message::text(r#"{"type":"auth_ok"}"#))
                    .await
                    .unwrap();
                ws.close(None).await.unwrap();
            });
        }
    });

    TlsServer {
        url: format!("wss://localhost:{port}"),
        ca_pem: ca_cert.pem(),
    }
}
