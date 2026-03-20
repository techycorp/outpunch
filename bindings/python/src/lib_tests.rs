use super::*;

#[test]
fn to_rust_converts_durations() {
    let config = ClientConfig::new(
        "ws://localhost:3000/ws".to_string(),
        "secret".to_string(),
        "svc".to_string(),
        "http://localhost:8080".to_string(),
        2.5,
        10.0,
    );
    let rust = config.to_rust();
    assert_eq!(rust.reconnect_delay, Duration::from_millis(2500));
    assert_eq!(rust.request_timeout, Duration::from_secs(10));
}

#[test]
fn to_rust_copies_all_fields() {
    let config = ClientConfig::new(
        "wss://example.com/ws".to_string(),
        "my-secret".to_string(),
        "my-app".to_string(),
        "http://localhost:9000".to_string(),
        5.0,
        25.0,
    );
    let rust = config.to_rust();
    assert_eq!(rust.server_url, "wss://example.com/ws");
    assert_eq!(rust.secret, "my-secret");
    assert_eq!(rust.service, "my-app");
    assert_eq!(rust.forward_to, "http://localhost:9000");
}

#[test]
fn repr_includes_key_fields() {
    let config = ClientConfig::new(
        "ws://x".to_string(),
        "s".to_string(),
        "svc".to_string(),
        "http://localhost:8080".to_string(),
        5.0,
        25.0,
    );
    let r = config.__repr__();
    assert!(r.contains("ws://x"));
    assert!(r.contains("svc"));
    assert!(r.contains("http://localhost:8080"));
}
