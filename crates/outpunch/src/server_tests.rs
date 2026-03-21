use super::*;
use crate::protocol::AuthMessage;

fn test_config() -> ServerConfig {
    ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    }
}

/// Helper: set up on_message callback that collects into a channel.
fn collect_messages(connection: &Connection) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(64);
    connection.on_message(move |msg| {
        let _ = tx.try_send(msg);
    });
    rx
}

fn auth_json(secret: &str, service: &str) -> String {
    serde_json::to_string(&AuthMessage {
        msg_type: "auth".to_string(),
        token: secret.to_string(),
        service: service.to_string(),
    })
    .unwrap()
}

#[test]
fn constant_time_eq_matching() {
    assert!(constant_time_eq("secret", "secret"));
}

#[test]
fn constant_time_eq_not_matching() {
    assert!(!constant_time_eq("secret", "wrong"));
}

#[test]
fn constant_time_eq_different_lengths() {
    assert!(!constant_time_eq("short", "longer"));
}

#[test]
fn constant_time_eq_empty() {
    assert!(constant_time_eq("", ""));
}

#[tokio::test]
async fn handle_request_no_client_returns_502() {
    let server = OutpunchServer::new(test_config());

    let resp = server
        .handle_request(IncomingRequest {
            service: "my-service".to_string(),
            method: "GET".to_string(),
            path: "test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        })
        .await;

    assert_eq!(resp.status, 502);
    assert!(resp.body.as_ref().unwrap().contains("no client connected"));
}

#[tokio::test]
async fn handle_request_timeout_returns_504() {
    let config = ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(50),
        ..ServerConfig::default()
    };
    let server = OutpunchServer::new(config);

    // Register a fake service handle that never responds
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), ServiceHandle { outgoing_tx });
    }

    // Drain the outgoing channel so send doesn't block
    tokio::spawn(async move { while outgoing_rx.recv().await.is_some() {} });

    let resp = server
        .handle_request(IncomingRequest {
            service: "my-service".to_string(),
            method: "GET".to_string(),
            path: "test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        })
        .await;

    assert_eq!(resp.status, 504);
    assert!(resp.body.as_ref().unwrap().contains("timeout"));
}

#[tokio::test]
async fn full_request_response_cycle() {
    let server = OutpunchServer::new(test_config());

    // Set up a fake service handle
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), ServiceHandle { outgoing_tx });
    }

    let server_clone = server.clone();

    // Simulate the client: read request, send response
    let client_handle = tokio::spawn(async move {
        let raw = outgoing_rx.recv().await.unwrap();
        let msg = protocol::parse_message(&raw).unwrap();

        if let Message::Request(req) = msg {
            let resp = TunnelResponse {
                msg_type: "response".to_string(),
                request_id: req.request_id.clone(),
                status: 200,
                headers: HashMap::new(),
                body: Some("hello from service".to_string()),
                body_encoding: None,
            };

            // Deliver the response directly to the pending map
            let mut state = server_clone.state.lock().await;
            if let Some(sender) = state.pending.remove(&req.request_id) {
                let _ = sender.send(resp);
            }
        }
    });

    let resp = server
        .handle_request(IncomingRequest {
            service: "my-service".to_string(),
            method: "GET".to_string(),
            path: "api/test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        })
        .await;

    client_handle.await.unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.as_deref(), Some("hello from service"));
}

#[tokio::test]
async fn auth_valid_token_registers_service() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;

    // Close to end the connection loop after auth
    connection.close();

    connection.run().await;

    // Should have received auth_ok
    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    assert!(matches!(msg, Message::AuthOk(_)));
}

#[tokio::test]
async fn auth_invalid_token_rejects() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    connection
        .push_message(auth_json("wrong-secret", "my-service"))
        .await;

    connection.close();

    connection.run().await;

    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    assert!(matches!(msg, Message::AuthError(_)));

    // Service should NOT be registered
    assert!(!server.is_connected("my-service").await);
}

#[tokio::test]
async fn connection_cleanup_removes_service() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let _outgoing_rx = collect_messages(&connection);

    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;

    // Close to simulate disconnect
    connection.close();

    connection.run().await;

    // After connection ends, service should be removed
    assert!(!server.is_connected("my-service").await);
}

#[tokio::test]
async fn connection_routes_response_to_pending_request() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    // Auth
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;

    let conn_clone = connection.clone();
    let conn_handle = tokio::spawn(async move {
        conn_clone.run().await;
    });

    // Wait for auth_ok
    let _ = outgoing_rx.recv().await.unwrap();

    // Now make a request
    let server_clone = server.clone();
    let request_handle = tokio::spawn(async move {
        server_clone
            .handle_request(IncomingRequest {
                service: "my-service".to_string(),
                method: "GET".to_string(),
                path: "test".to_string(),
                query: HashMap::new(),
                headers: HashMap::new(),
                body: None,
            })
            .await
    });

    // Read the request that was sent to the "client"
    let raw_req = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw_req).unwrap();
    let request_id = if let Message::Request(req) = msg {
        req.request_id
    } else {
        panic!("expected Request");
    };

    // Send response back through the connection
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: request_id.clone(),
        status: 200,
        headers: HashMap::new(),
        body: Some("it works".to_string()),
        body_encoding: None,
    };
    connection
        .push_message(protocol::serialize_response(&resp))
        .await;

    // The request should complete
    let result = request_handle.await.unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body.as_deref(), Some("it works"));

    // Clean up
    connection.close();
    conn_handle.await.unwrap();
}

#[tokio::test]
async fn handle_request_returns_502_when_client_channel_closed() {
    let server = OutpunchServer::new(test_config());

    let (outgoing_tx, outgoing_rx) = mpsc::channel(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), ServiceHandle { outgoing_tx });
    }

    // Drop the receiver — simulates the bridge loop dying
    drop(outgoing_rx);

    let resp = server
        .handle_request(IncomingRequest {
            service: "my-service".to_string(),
            method: "GET".to_string(),
            path: "test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        })
        .await;

    assert_eq!(resp.status, 502);
    assert!(resp.body.as_ref().unwrap().contains("connection lost"));
}

#[tokio::test]
async fn handle_request_returns_502_when_oneshot_sender_dropped() {
    let config = ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    };
    let server = OutpunchServer::new(config);

    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), ServiceHandle { outgoing_tx });
    }

    let server_clone = server.clone();

    // Receive the request, then drop the pending sender (simulate disconnect)
    tokio::spawn(async move {
        let _ = outgoing_rx.recv().await;
        let mut state = server_clone.state.lock().await;
        state.pending.clear();
    });

    let resp = server
        .handle_request(IncomingRequest {
            service: "my-service".to_string(),
            method: "GET".to_string(),
            path: "test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        })
        .await;

    assert_eq!(resp.status, 502);
    assert!(resp.body.as_ref().unwrap().contains("disconnected"));
}

#[tokio::test]
async fn connection_ignores_malformed_messages() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    // Auth
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;

    let conn_clone = connection.clone();
    let conn_handle = tokio::spawn(async move {
        conn_clone.run().await;
    });

    // Wait for auth_ok
    let _ = outgoing_rx.recv().await.unwrap();

    // Send garbage
    connection.push_message("not json at all".to_string()).await;
    connection
        .push_message(r#"{"type": "unknown_thing"}"#.to_string())
        .await;

    // Now send a real request and verify the server still works
    let server_clone = server.clone();
    let request_handle = tokio::spawn(async move {
        server_clone
            .handle_request(IncomingRequest {
                service: "my-service".to_string(),
                method: "GET".to_string(),
                path: "still-works".to_string(),
                query: HashMap::new(),
                headers: HashMap::new(),
                body: None,
            })
            .await
    });

    // Read the forwarded request
    let raw_req = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw_req).unwrap();
    let request_id = match msg {
        Message::Request(req) => {
            assert_eq!(req.path, "still-works");
            req.request_id
        }
        _ => panic!("expected Request"),
    };

    // Send response
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id,
        status: 200,
        headers: HashMap::new(),
        body: Some("survived garbage".to_string()),
        body_encoding: None,
    };
    connection
        .push_message(protocol::serialize_response(&resp))
        .await;

    let result = request_handle.await.unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body.as_deref(), Some("survived garbage"));

    connection.close();
    conn_handle.await.unwrap();
}

#[tokio::test]
async fn connection_rejects_non_auth_first_message() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    // Send a request message instead of auth
    connection
        .push_message(r#"{"type": "response", "request_id": "abc", "status": 200}"#.to_string())
        .await;

    connection.close();

    connection.run().await;

    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    match msg {
        Message::AuthError(e) => assert!(e.message.contains("expected auth")),
        other => panic!("expected AuthError, got: {other:?}"),
    }

    // Service should not be registered
    assert!(!server.is_connected("my-service").await);
}
