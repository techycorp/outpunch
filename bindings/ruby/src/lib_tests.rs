use super::ClientConfig;

#[test]
fn client_config_defaults() {
    let config = ClientConfig::new(
        "wss://example.com/ws".into(),
        "secret".into(),
        "my-service".into(),
        None,
        None,
        None,
    );
    let rust = config.to_rust();
    assert_eq!(rust.forward_to, "http://localhost:8080");
    assert_eq!(rust.reconnect_delay.as_secs_f64(), 5.0);
    assert_eq!(rust.request_timeout.as_secs_f64(), 25.0);
}

#[test]
fn client_config_custom_values() {
    let config = ClientConfig::new(
        "wss://example.com/ws".into(),
        "mysecret".into(),
        "stormsnap".into(),
        Some("http://localhost:8081".into()),
        Some(10.0),
        Some(30.0),
    );
    let rust = config.to_rust();
    assert_eq!(rust.server_url, "wss://example.com/ws");
    assert_eq!(rust.secret, "mysecret");
    assert_eq!(rust.service, "stormsnap");
    assert_eq!(rust.forward_to, "http://localhost:8081");
    assert_eq!(rust.reconnect_delay.as_secs_f64(), 10.0);
    assert_eq!(rust.request_timeout.as_secs_f64(), 30.0);
}
