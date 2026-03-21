// Rust-side tests are minimal — the real tests are in JS (tests/spike.test.ts).
// These just verify the type conversions compile correctly.

#[test]
fn server_config_defaults() {
    let config = super::ServerConfig {
        secret: "test".to_string(),
        timeout_ms: None,
        max_body_size: None,
    };
    assert_eq!(config.secret, "test");
    assert!(config.timeout_ms.is_none());
}
