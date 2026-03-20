use super::*;
use crate::protocol::AuthMessage;

fn test_config() -> ServerConfig {
    ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    }
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

    // Register a fake connection that never responds
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), Connection { outgoing_tx });
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

    // Set up a fake client connection
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), Connection { outgoing_tx });
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

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);

    // Send auth message
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "test-secret".to_string(),
        service: "my-service".to_string(),
    };
    incoming_tx
        .send(serde_json::to_string(&auth).unwrap())
        .await
        .unwrap();

    // Drop sender to end the connection loop
    drop(incoming_tx);

    server.handle_connection(incoming_rx, outgoing_tx).await;

    // Should have received auth_ok
    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    assert!(matches!(msg, Message::AuthOk(_)));
}

#[tokio::test]
async fn auth_invalid_token_rejects() {
    let server = OutpunchServer::new(test_config());

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);

    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "wrong-secret".to_string(),
        service: "my-service".to_string(),
    };
    incoming_tx
        .send(serde_json::to_string(&auth).unwrap())
        .await
        .unwrap();

    drop(incoming_tx);

    server.handle_connection(incoming_rx, outgoing_tx).await;

    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    assert!(matches!(msg, Message::AuthError(_)));

    // Service should NOT be registered
    assert!(!server.is_connected("my-service").await);
}

#[tokio::test]
async fn connection_cleanup_removes_service() {
    let server = OutpunchServer::new(test_config());

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, _outgoing_rx) = mpsc::channel(16);

    // Auth
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "test-secret".to_string(),
        service: "my-service".to_string(),
    };
    incoming_tx
        .send(serde_json::to_string(&auth).unwrap())
        .await
        .unwrap();

    // Drop to simulate disconnect
    drop(incoming_tx);

    server.handle_connection(incoming_rx, outgoing_tx).await;

    // After connection ends, service should be removed
    assert!(!server.is_connected("my-service").await);
}

#[tokio::test]
async fn handle_connection_routes_response_to_pending_request() {
    let server = OutpunchServer::new(test_config());

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);

    // Auth
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "test-secret".to_string(),
        service: "my-service".to_string(),
    };
    incoming_tx
        .send(serde_json::to_string(&auth).unwrap())
        .await
        .unwrap();

    let server_clone = server.clone();
    let conn_handle = tokio::spawn(async move {
        server_clone
            .handle_connection(incoming_rx, outgoing_tx)
            .await;
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
    incoming_tx
        .send(protocol::serialize_response(&resp))
        .await
        .unwrap();

    // The request should complete
    let result = request_handle.await.unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body.as_deref(), Some("it works"));

    // Clean up
    drop(incoming_tx);
    conn_handle.await.unwrap();
}

#[tokio::test]
async fn handle_request_returns_502_when_client_channel_closed() {
    // Simulates the client's outgoing channel being dropped (e.g., client crashed)
    // while the server tries to send a request through it.
    let server = OutpunchServer::new(test_config());

    let (outgoing_tx, outgoing_rx) = mpsc::channel(16);
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("my-service".to_string(), Connection { outgoing_tx });
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
    // Simulates the client disconnecting after accepting the request
    // but before sending a response — the oneshot sender gets dropped.
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
            .insert("my-service".to_string(), Connection { outgoing_tx });
    }

    let server_clone = server.clone();

    // Receive the request, then drop the pending sender (simulate disconnect)
    tokio::spawn(async move {
        let _ = outgoing_rx.recv().await;
        // Drop the oneshot sender by removing the pending entry
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
async fn handle_connection_ignores_malformed_messages() {
    // A connected client sends garbage after auth — server should
    // ignore it and keep listening, not crash.
    let server = OutpunchServer::new(test_config());

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);

    // Auth
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "test-secret".to_string(),
        service: "my-service".to_string(),
    };
    incoming_tx
        .send(serde_json::to_string(&auth).unwrap())
        .await
        .unwrap();

    let server_clone = server.clone();
    let conn_handle = tokio::spawn(async move {
        server_clone
            .handle_connection(incoming_rx, outgoing_tx)
            .await;
    });

    // Wait for auth_ok
    let _ = outgoing_rx.recv().await.unwrap();

    // Send garbage
    incoming_tx
        .send("not json at all".to_string())
        .await
        .unwrap();
    incoming_tx
        .send(r#"{"type": "unknown_thing"}"#.to_string())
        .await
        .unwrap();

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
    incoming_tx
        .send(protocol::serialize_response(&resp))
        .await
        .unwrap();

    let result = request_handle.await.unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body.as_deref(), Some("survived garbage"));

    drop(incoming_tx);
    conn_handle.await.unwrap();
}

#[tokio::test]
async fn handle_connection_rejects_non_auth_first_message() {
    // If the first message isn't an auth message, the server should
    // send auth_error and close the connection.
    let server = OutpunchServer::new(test_config());

    let (incoming_tx, incoming_rx) = mpsc::channel(16);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);

    // Send a request message instead of auth
    let bad_first_msg = r#"{"type": "response", "request_id": "abc", "status": 200}"#;
    incoming_tx.send(bad_first_msg.to_string()).await.unwrap();

    drop(incoming_tx);

    server.handle_connection(incoming_rx, outgoing_tx).await;

    let raw = outgoing_rx.recv().await.unwrap();
    let msg = protocol::parse_message(&raw).unwrap();
    match msg {
        Message::AuthError(e) => assert!(e.message.contains("expected auth")),
        other => panic!("expected AuthError, got: {other:?}"),
    }

    // Service should not be registered
    assert!(!server.is_connected("my-service").await);
}
