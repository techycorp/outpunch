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

/// A service handle not backed by a real connection; the test owns the receiver.
fn fake_handle(outgoing_tx: mpsc::Sender<String>) -> ServiceHandle {
    ServiceHandle {
        conn_id: 0,
        outgoing_tx,
        connection: std::sync::Weak::new(),
    }
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
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
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
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
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
            if let Some(pending) = state.pending.remove(&req.request_id) {
                let _ = pending.tx.send(resp);
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
async fn stale_disconnect_does_not_remove_newer_connection() {
    let server = OutpunchServer::new(test_config());

    let old = server.create_connection();
    let mut old_rx = collect_messages(&old);
    let old_run = tokio::spawn({
        let old = old.clone();
        async move { old.run().await }
    });
    old.push_message(auth_json("test-secret", "my-service"))
        .await;
    assert!(matches!(
        protocol::parse_message(&old_rx.recv().await.unwrap()).unwrap(),
        Message::AuthOk(_)
    ));

    let new = server.create_connection();
    let mut new_rx = collect_messages(&new);
    let new_run = tokio::spawn({
        let new = new.clone();
        async move { new.run().await }
    });
    new.push_message(auth_json("test-secret", "my-service"))
        .await;
    assert!(matches!(
        protocol::parse_message(&new_rx.recv().await.unwrap()).unwrap(),
        Message::AuthOk(_)
    ));

    old.close();
    old_run.await.unwrap();

    assert!(server.is_connected("my-service").await);

    new.close();
    new_run.await.unwrap();
}

#[tokio::test]
async fn disconnect_mid_request_fails_fast_with_502() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    let conn_run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    let _ = outgoing_rx.recv().await.unwrap();

    let request_handle = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .handle_request(IncomingRequest {
                    service: "my-service".to_string(),
                    method: "GET".to_string(),
                    path: "test".to_string(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    body: None,
                })
                .await
        }
    });
    let _ = outgoing_rx.recv().await.unwrap();

    connection.close();
    conn_run.await.unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(1), request_handle)
        .await
        .expect("request should fail fast after disconnect, not wait for timeout")
        .unwrap();
    assert_eq!(resp.status, 502);
}

#[tokio::test]
async fn stalled_service_does_not_block_other_services() {
    let server = OutpunchServer::new(test_config());

    let (outgoing_tx, _outgoing_rx) = mpsc::channel::<String>(1);
    outgoing_tx.send("fill".to_string()).await.unwrap();
    {
        let mut state = server.state.lock().await;
        state
            .services
            .insert("stalled".to_string(), fake_handle(outgoing_tx));
    }

    let stalled = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .handle_request(IncomingRequest {
                    service: "stalled".to_string(),
                    method: "GET".to_string(),
                    path: "test".to_string(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    body: None,
                })
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let other = tokio::time::timeout(
        Duration::from_secs(1),
        server.handle_request(IncomingRequest {
            service: "other".to_string(),
            method: "GET".to_string(),
            path: "test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: None,
        }),
    )
    .await
    .expect("request to another service should not wait on the stalled one");
    assert_eq!(other.status, 502);

    stalled.abort();
}

#[tokio::test(start_paused = true)]
async fn connection_without_auth_is_closed() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let _outgoing_rx = collect_messages(&connection);

    tokio::time::timeout(Duration::from_secs(60), connection.run())
        .await
        .expect("a connection that never authenticates should be closed");
}

#[tokio::test]
async fn client_cannot_answer_another_services_request() {
    let server = OutpunchServer::new(test_config());

    let victim = server.create_connection();
    let mut victim_rx = collect_messages(&victim);
    let victim_run = tokio::spawn({
        let victim = victim.clone();
        async move { victim.run().await }
    });
    victim
        .push_message(auth_json("test-secret", "victim"))
        .await;
    let _ = victim_rx.recv().await.unwrap();

    let attacker = server.create_connection();
    let mut attacker_rx = collect_messages(&attacker);
    let attacker_run = tokio::spawn({
        let attacker = attacker.clone();
        async move { attacker.run().await }
    });
    attacker
        .push_message(auth_json("test-secret", "attacker"))
        .await;
    let _ = attacker_rx.recv().await.unwrap();

    let request_handle = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .handle_request(IncomingRequest {
                    service: "victim".to_string(),
                    method: "GET".to_string(),
                    path: "test".to_string(),
                    query: HashMap::new(),
                    headers: HashMap::new(),
                    body: None,
                })
                .await
        }
    });
    let raw = victim_rx.recv().await.unwrap();
    let Message::Request(req) = protocol::parse_message(&raw).unwrap() else {
        panic!("expected request");
    };

    let response = |status: u16| {
        serde_json::to_string(&TunnelResponse {
            msg_type: "response".to_string(),
            request_id: req.request_id.clone(),
            status,
            headers: HashMap::new(),
            body: None,
            body_encoding: None,
        })
        .unwrap()
    };
    attacker.push_message(response(418)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    victim.push_message(response(200)).await;

    let resp = request_handle.await.unwrap();
    assert_eq!(resp.status, 200);

    victim.close();
    attacker.close();
    victim_run.await.unwrap();
    attacker_run.await.unwrap();
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
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
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
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
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

fn plain_request(service: &str) -> IncomingRequest {
    IncomingRequest {
        service: service.to_string(),
        method: "GET".to_string(),
        path: "test".to_string(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: None,
    }
}

#[tokio::test]
async fn try_handle_request_not_connected() {
    let server = OutpunchServer::new(test_config());

    let err = server
        .try_handle_request(plain_request("my-service"))
        .await
        .unwrap_err();

    assert_eq!(err, TunnelError::NotConnected);
    assert_eq!(err.status(), 502);
}

#[tokio::test]
async fn try_handle_request_timeout() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(50),
        ..ServerConfig::default()
    });
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);
    server
        .state
        .lock()
        .await
        .services
        .insert("my-service".to_string(), fake_handle(outgoing_tx));
    tokio::spawn(async move { while outgoing_rx.recv().await.is_some() {} });

    let err = server
        .try_handle_request(plain_request("my-service"))
        .await
        .unwrap_err();

    assert_eq!(err, TunnelError::Timeout);
    assert_eq!(err.status(), 504);
}

#[tokio::test]
async fn auth_rejects_empty_token_when_secret_unset() {
    let server = OutpunchServer::new(ServerConfig::default());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);

    connection.push_message(auth_json("", "my-service")).await;
    connection.close();
    connection.run().await;

    let msg = protocol::parse_message(&outgoing_rx.recv().await.unwrap()).unwrap();
    assert!(matches!(msg, Message::AuthError(_)));
}

#[tokio::test]
async fn newer_connection_replaces_and_closes_older_one() {
    let server = OutpunchServer::new(test_config());

    let old = server.create_connection();
    let mut old_rx = collect_messages(&old);
    let old_run = tokio::spawn({
        let old = old.clone();
        async move { old.run().await }
    });
    old.push_message(auth_json("test-secret", "my-service"))
        .await;
    let _ = old_rx.recv().await.unwrap();

    let new = server.create_connection();
    let mut new_rx = collect_messages(&new);
    let new_run = tokio::spawn({
        let new = new.clone();
        async move { new.run().await }
    });
    new.push_message(auth_json("test-secret", "my-service"))
        .await;
    let _ = new_rx.recv().await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), old_run)
        .await
        .expect("replaced connection should be closed")
        .unwrap();
    assert!(server.is_connected("my-service").await);

    new.close();
    new_run.await.unwrap();
}

#[tokio::test]
async fn send_to_stalled_client_respects_timeout() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(100),
        ..ServerConfig::default()
    });
    let (outgoing_tx, _outgoing_rx) = mpsc::channel::<String>(1);
    outgoing_tx.send("fill".to_string()).await.unwrap();
    server
        .state
        .lock()
        .await
        .services
        .insert("stalled".to_string(), fake_handle(outgoing_tx));

    let err = tokio::time::timeout(
        Duration::from_secs(1),
        server.try_handle_request(plain_request("stalled")),
    )
    .await
    .expect("send to a stalled client should time out")
    .unwrap_err();

    assert_eq!(err, TunnelError::Timeout);
    assert!(server.state.lock().await.pending.is_empty());
}
