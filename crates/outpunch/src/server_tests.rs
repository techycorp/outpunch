use super::*;
use crate::protocol::AuthMessage;

use crate::test_capture as capture;

impl capture::Captured {
    /// Waits (up to 1 s) for an event with this message and field, for events logged by a task
    /// that ends after the test's last await.
    async fn wait_for(&self, message: &str, field: (&str, &str)) -> capture::Event {
        for _ in 0..200 {
            if let Some(event) = self
                .events()
                .into_iter()
                .find(|e| e.message() == message && e.field(field.0) == Some(field.1))
            {
                return event;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("no {message:?} with {field:?} in {:#?}", self.events())
    }
}

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
        let mut state = server.state.lock().unwrap();
        state
            .services
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
    }

    // Drain the outgoing channel so send doesn't block
    tokio::spawn(async move { while outgoing_rx.recv().await.is_some() {} });

    let (logs, _guard) = capture::start();
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
    let event = logs.find("request timed out");
    assert_eq!(event.level, tracing::Level::WARN);
    assert_eq!(event.field("service"), Some("my-service"));
    assert_eq!(event.field("req"), Some(short_id(&resp.request_id)));
}

#[tokio::test]
async fn full_request_response_cycle() {
    let server = OutpunchServer::new(test_config());

    // Set up a fake service handle
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(16);
    {
        let mut state = server.state.lock().unwrap();
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
            let mut state = server_clone.state.lock().unwrap();
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
        let mut state = server.state.lock().unwrap();
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
        let mut state = server.state.lock().unwrap();
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
        let mut state = server.state.lock().unwrap();
        state
            .services
            .insert("my-service".to_string(), fake_handle(outgoing_tx));
    }

    let server_clone = server.clone();

    // Receive the request, then drop the pending sender (simulate disconnect)
    tokio::spawn(async move {
        let _ = outgoing_rx.recv().await;
        let mut state = server_clone.state.lock().unwrap();
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
        .unwrap()
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
        .unwrap()
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
    assert!(server.state.lock().unwrap().pending.is_empty());
}

/// Waits until the pending map holds `n` entries.
async fn wait_for_pending(server: &OutpunchServer, n: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.state.lock().unwrap().pending.len() != n {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("pending map should reach the expected size");
}

// Regression tests for F1 in spec/FINDINGS.md: a cancelled request must not leak its pending entry.
#[tokio::test]
async fn cancelled_request_while_send_blocked_removes_pending() {
    let server = OutpunchServer::new(test_config());
    let (outgoing_tx, _outgoing_rx) = mpsc::channel::<String>(1);
    outgoing_tx.send("fill".to_string()).await.unwrap();
    server
        .state
        .lock()
        .unwrap()
        .services
        .insert("stalled".to_string(), fake_handle(outgoing_tx));

    let request = tokio::spawn({
        let server = server.clone();
        async move { server.handle_request(plain_request("stalled")).await }
    });
    wait_for_pending(&server, 1).await;

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    assert!(server.state.lock().unwrap().pending.is_empty());
}

#[tokio::test]
async fn cancelled_request_awaiting_response_removes_pending() {
    let server = OutpunchServer::new(test_config());
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(16);
    server
        .state
        .lock()
        .unwrap()
        .services
        .insert("silent".to_string(), fake_handle(outgoing_tx));

    let request = tokio::spawn({
        let server = server.clone();
        async move { server.handle_request(plain_request("silent")).await }
    });
    // The request reaches the client, which never answers.
    let _ = outgoing_rx.recv().await.unwrap();

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    assert!(server.state.lock().unwrap().pending.is_empty());
}

// F3 in spec/FINDINGS.md (spec/RunDrop.cfg, spec/RunDropLive.cfg): dropping the run()
// future still runs cleanup_connection.
#[tokio::test]
async fn dropped_run_unregisters_and_fails_pending() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    let run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    let _ = outgoing_rx.recv().await.unwrap();

    let request = tokio::spawn({
        let server = server.clone();
        async move { server.try_handle_request(plain_request("my-service")).await }
    });
    let _ = outgoing_rx.recv().await.unwrap();

    run.abort();
    let _ = run.await;

    let result = tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .expect("in-flight request should fail fast, not wait for the timeout")
        .unwrap();
    assert_eq!(result.unwrap_err(), TunnelError::Disconnected);
    assert!(!server.is_connected("my-service").await);
    let closed = logs.find("connection closed");
    assert_eq!(
        (closed.field("reason"), closed.field("failed")),
        (Some("dropped"), Some("1"))
    );
}

// F4 in spec/FINDINGS.md (spec/DrainDeath.cfg): if the on_message callback panics, the
// drain task dies and the connection closes, so the service is unregistered.
#[tokio::test]
async fn dead_callback_unregisters_service() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(200),
        ..ServerConfig::default()
    });
    let connection = server.create_connection();
    let (tx, mut rx) = mpsc::channel::<String>(64);
    connection.on_message(move |msg| {
        if msg.contains(r#""type":"request""#) {
            panic!("callback panic (F4 test)");
        }
        let _ = tx.try_send(msg);
    });
    let _run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    let _ = rx.recv().await.unwrap();

    let _ = server.try_handle_request(plain_request("my-service")).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.is_connected("my-service").await {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the service is still registered after the callback panicked");
    assert_eq!(
        server
            .try_handle_request(plain_request("my-service"))
            .await
            .unwrap_err(),
        TunnelError::NotConnected
    );
    let panicked = logs.find("outgoing message callback panicked; closing connection");
    assert_eq!(panicked.level, tracing::Level::ERROR);
    assert_eq!(panicked.field("service"), Some("my-service"));
    logs.wait_for("connection closed", ("reason", "drain_died"))
        .await;
}

/// Reads the next outgoing message, which must be a tunnel request, and returns its id.
async fn next_request_id(outgoing_rx: &mut mpsc::Receiver<String>) -> String {
    match protocol::parse_message(&outgoing_rx.recv().await.unwrap()).unwrap() {
        Message::Request(req) => req.request_id,
        other => panic!("expected a request, got: {other:?}"),
    }
}

fn ok_response(request_id: &str) -> String {
    protocol::serialize_response(&TunnelResponse {
        msg_type: "response".to_string(),
        request_id: request_id.to_string(),
        status: 200,
        headers: HashMap::new(),
        body: None,
        body_encoding: None,
    })
}

#[tokio::test(start_paused = true)]
async fn request_times_out_over_a_real_connection() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(100),
        ..ServerConfig::default()
    });
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    let run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    let _auth_ok = outgoing_rx.recv().await.unwrap();

    // The client receives the request and never answers.
    let err = server
        .try_handle_request(plain_request("my-service"))
        .await
        .unwrap_err();
    assert_eq!(err, TunnelError::Timeout);
    assert!(server.state.lock().unwrap().pending.is_empty());

    connection.close();
    run.await.unwrap();
}

#[tokio::test]
async fn connection_keeps_serving_after_a_request_is_cancelled() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    let run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    let _auth_ok = outgoing_rx.recv().await.unwrap();

    let cancelled = tokio::spawn({
        let server = server.clone();
        async move { server.try_handle_request(plain_request("my-service")).await }
    });
    let _ = next_request_id(&mut outgoing_rx).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert!(server.state.lock().unwrap().pending.is_empty());

    let answered = tokio::spawn({
        let server = server.clone();
        async move { server.try_handle_request(plain_request("my-service")).await }
    });
    let request_id = next_request_id(&mut outgoing_rx).await;
    connection.push_message(ok_response(&request_id)).await;
    assert_eq!(answered.await.unwrap().unwrap().status, 200);

    connection.close();
    run.await.unwrap();
}

// F5 in spec/FINDINGS.md: adapters drop an outgoing request once its caller has an answer.
#[tokio::test]
async fn request_is_pending_until_its_caller_is_answered() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(50),
        ..ServerConfig::default()
    });
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(16);
    server
        .state
        .lock()
        .unwrap()
        .services
        .insert("my-service".to_string(), fake_handle(outgoing_tx));

    let caller = {
        let server = server.clone();
        tokio::spawn(async move {
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
        })
    };
    let msg = outgoing_rx.recv().await.unwrap();
    assert!(server.is_pending(&msg));

    assert_eq!(caller.await.unwrap().status, 504);
    assert!(!server.is_pending(&msg));
}

#[test]
fn messages_other_than_requests_are_always_pending() {
    let server = OutpunchServer::new(test_config());
    assert!(server.is_pending(r#"{"type":"auth_ok"}"#));
    assert!(server.is_pending(r#"{"type":"auth_error","message":"invalid token"}"#));
    assert!(server.is_pending("not json"));
    assert!(!server.is_pending(
        r#"{"type":"request","request_id":"unknown","method":"GET","path":"","query":{},"headers":{}}"#
    ));
}

#[test]
fn weak_connection_does_not_keep_the_connection_alive() {
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let weak = connection.downgrade();
    assert!(weak.upgrade().is_some());
    drop(connection);
    assert!(weak.upgrade().is_none());
}

fn idle_config(idle: Duration) -> ServerConfig {
    ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(600),
        client_idle_timeout: idle,
        ..ServerConfig::default()
    }
}

/// Authenticates a connection for "my-service" and returns it with its outgoing messages.
async fn authed(server: &OutpunchServer) -> (Connection, mpsc::Receiver<String>) {
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection
        .push_message(auth_json("test-secret", "my-service"))
        .await;
    assert!(outgoing_rx.recv().await.unwrap().contains("auth_ok"));
    (connection, outgoing_rx)
}

// R30 in spec/REQUIREMENTS.md: a client that sends nothing, not even a WebSocket ping, for
// client_idle_timeout is treated as gone. Its in-flight requests fail fast with 502 rather than
// waiting for the request timeout.
#[tokio::test(start_paused = true)]
async fn silent_client_is_evicted_after_the_idle_timeout() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(idle_config(Duration::from_secs(90)));
    let (_connection, mut outgoing_rx) = authed(&server).await;

    let request = tokio::spawn({
        let server = server.clone();
        async move { server.try_handle_request(plain_request("my-service")).await }
    });
    let _ = next_request_id(&mut outgoing_rx).await;

    tokio::time::sleep(Duration::from_secs(89)).await;
    assert!(server.is_connected("my-service").await);

    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!server.is_connected("my-service").await);
    assert_eq!(
        request.await.unwrap().unwrap_err(),
        TunnelError::Disconnected
    );
    let evicted = logs.find("client silent; evicting");
    assert_eq!(evicted.level, tracing::Level::WARN);
    assert_eq!(
        (evicted.field("service"), evicted.field("idle_s")),
        (Some("my-service"), Some("90"))
    );
    let closed = logs.find("connection closed");
    assert_eq!(
        (
            closed.field("reason"),
            closed.field("failed"),
            closed.field("duration_s")
        ),
        (Some("idle"), Some("1"), Some("90"))
    );
}

#[tokio::test(start_paused = true)]
async fn heartbeats_keep_a_quiet_client_registered() {
    let server = OutpunchServer::new(idle_config(Duration::from_secs(90)));
    let (connection, _outgoing_rx) = authed(&server).await;

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_secs(30)).await;
        connection.heartbeat();
    }
    assert!(server.is_connected("my-service").await);
}

#[tokio::test(start_paused = true)]
async fn messages_keep_a_client_registered() {
    let server = OutpunchServer::new(idle_config(Duration::from_secs(90)));
    let (connection, _outgoing_rx) = authed(&server).await;

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_secs(30)).await;
        connection.push_message("junk".to_string()).await;
    }
    assert!(server.is_connected("my-service").await);
}

// docs/logging.md, server events.
#[tokio::test]
async fn registration_and_replacement_are_logged() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(test_config());
    let (first, _first_rx) = authed(&server).await;
    let registered = logs.find("service registered");
    assert_eq!(
        (
            registered.level,
            registered.field("service"),
            registered.field("conn")
        ),
        (tracing::Level::INFO, Some("my-service"), Some("1"))
    );
    assert_eq!(logs.find("client authenticated").field("conn"), Some("1"));

    let (_second, _second_rx) = authed(&server).await;
    let replaced = logs.find("service registered; older connection replaced");
    assert_eq!(
        (
            replaced.level,
            replaced.field("conn"),
            replaced.field("replaced")
        ),
        (tracing::Level::WARN, Some("2"), Some("1"))
    );
    let closed = logs.wait_for("connection closed", ("conn", "1")).await;
    assert_eq!(closed.field("reason"), Some("replaced"));
    assert_eq!(first.id(), 1);
}

#[tokio::test]
async fn a_closed_connection_is_logged_with_reason_closed() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(test_config());
    let (connection, _rx) = authed(&server).await;
    assert_eq!(connection.service().as_deref(), Some("my-service"));
    connection.close();
    let closed = logs.wait_for("connection closed", ("conn", "1")).await;
    assert_eq!(
        (closed.level, closed.field("reason"), closed.field("failed")),
        (tracing::Level::INFO, Some("closed"), Some("0"))
    );
}

/// Runs a connection that sends `first` as its first message; returns the rejection event.
async fn auth_rejection(config: ServerConfig, first: String) -> capture::Event {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(config);
    let connection = server.create_connection();
    let mut outgoing_rx = collect_messages(&connection);
    let run = tokio::spawn({
        let connection = connection.clone();
        async move { connection.run().await }
    });
    connection.push_message(first).await;
    assert!(outgoing_rx.recv().await.unwrap().contains("auth_error"));
    run.await.unwrap();
    assert!(
        logs.events()
            .iter()
            .all(|e| e.fields.values().all(|v| !v.contains("test-secret"))),
        "a secret was logged: {:#?}",
        logs.events()
    );
    logs.find("auth rejected")
}

#[tokio::test]
async fn auth_rejections_are_logged_with_their_reason_and_without_the_token() {
    let wrong = auth_rejection(test_config(), auth_json("not-the-secret", "svc")).await;
    assert_eq!(
        (wrong.level, wrong.field("reason")),
        (tracing::Level::WARN, Some("invalid_token"))
    );
    let not_auth = auth_rejection(test_config(), "hello".to_string()).await;
    assert_eq!(not_auth.field("reason"), Some("not_auth"));
    let unset = auth_rejection(ServerConfig::default(), auth_json("test-secret", "svc")).await;
    assert_eq!(unset.field("reason"), Some("secret_not_set"));
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_never_authenticates_is_logged() {
    let (logs, _guard) = capture::start();
    let server = OutpunchServer::new(test_config());
    let connection = server.create_connection();
    let _rx = collect_messages(&connection);
    connection.run().await;
    let event = logs.find("auth not completed");
    assert_eq!(
        (event.level, event.field("reason")),
        (tracing::Level::INFO, Some("timeout"))
    );
}
