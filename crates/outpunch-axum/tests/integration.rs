use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use outpunch::protocol::{self, AuthMessage, Message};
use outpunch::server::{OutpunchServer, ServerConfig};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Start an axum server with outpunch routes, return the base URL.
async fn start_server(server: OutpunchServer) -> String {
    let app = outpunch_axum::router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Connect a simulated tunnel client via WebSocket, authenticate, and return the WS halves.
async fn connect_client(
    base_url: &str,
    secret: &str,
    service: &str,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMessage,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let ws_url = base_url.replace("http://", "ws://") + "/ws";
    let (ws, _) = connect_async(&ws_url).await.unwrap();
    let (mut sink, mut stream) = ws.split();

    // Auth
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: secret.to_string(),
        service: service.to_string(),
    };
    sink.send(WsMessage::text(serde_json::to_string(&auth).unwrap()))
        .await
        .unwrap();

    // Wait for auth_ok
    let resp = stream.next().await.unwrap().unwrap();
    let text = match resp {
        WsMessage::Text(t) => t,
        other => panic!("unexpected message: {other:?}"),
    };
    match protocol::parse_message(&text).unwrap() {
        Message::AuthOk(_) => {}
        other => panic!("expected auth_ok, got: {other:?}"),
    }

    (sink, stream)
}

/// Spawn a task that simulates a tunnel client: reads requests, responds with fixed status + body.
fn spawn_echo_client(
    mut sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMessage,
    >,
    mut stream: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                WsMessage::Text(t) => t,
                _ => continue,
            };

            match protocol::parse_message(&text) {
                Ok(Message::Request(req)) => {
                    let resp = protocol::TunnelResponse {
                        msg_type: "response".to_string(),
                        request_id: req.request_id,
                        status: 200,
                        headers: HashMap::from([(
                            "content-type".to_string(),
                            "text/plain".to_string(),
                        )]),
                        body: Some(format!("echo: {} /{}", req.method, req.path)),
                        body_encoding: None,
                    };
                    sink.send(WsMessage::text(protocol::serialize_response(&resp)))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
    })
}

#[tokio::test]
async fn full_round_trip() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let (sink, stream) = connect_client(&base_url, "test-secret", "my-service").await;
    let client_handle = spawn_echo_client(sink, stream);

    // Give the connection a moment to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Make an HTTP request through the tunnel
    let resp = reqwest::get(format!("{base_url}/tunnel/my-service/api/test"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "echo: GET /api/test");

    client_handle.abort();
}

#[tokio::test]
async fn returns_502_when_no_client_connected() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let resp = reqwest::get(format!("{base_url}/tunnel/my-service/api/test"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn returns_504_on_timeout() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_millis(100),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    // Connect a client that never responds
    let (_sink, _stream) = connect_client(&base_url, "test-secret", "my-service").await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("{base_url}/tunnel/my-service/api/test"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 504);
}

#[tokio::test]
async fn rejects_invalid_auth_token() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;
    let ws_url = base_url.replace("http://", "ws://") + "/ws";

    let (ws, _) = connect_async(&ws_url).await.unwrap();
    let (mut sink, mut stream) = ws.split();

    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "wrong-secret".to_string(),
        service: "my-service".to_string(),
    };
    sink.send(WsMessage::text(serde_json::to_string(&auth).unwrap()))
        .await
        .unwrap();

    // Server may send auth_error then close, or just close.
    // Either way, the service should not be registered.
    let mut got_auth_error = false;
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => match protocol::parse_message(&text) {
                Ok(Message::AuthError(e)) => {
                    assert!(e.message.contains("invalid token"));
                    got_auth_error = true;
                }
                _ => {}
            },
            Ok(WsMessage::Close(_)) => break,
            Err(_) => break,
            _ => continue,
        }
    }

    assert!(got_auth_error, "should have received auth_error");
}

#[tokio::test]
async fn forwards_post_with_body() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let (sink, stream) = connect_client(&base_url, "test-secret", "my-service").await;
    let client_handle = spawn_echo_client(sink, stream);

    tokio::time::sleep(Duration::from_millis(50)).await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base_url}/tunnel/my-service/api/submit"))
        .body("test data")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "echo: POST /api/submit");

    client_handle.abort();
}

#[tokio::test]
async fn different_http_methods() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let (sink, stream) = connect_client(&base_url, "test-secret", "my-service").await;
    let client_handle = spawn_echo_client(sink, stream);

    tokio::time::sleep(Duration::from_millis(50)).await;

    let http = reqwest::Client::new();

    for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        let resp = http
            .request(
                method.parse().unwrap(),
                format!("{base_url}/tunnel/my-service/test"),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(
            body.starts_with(&format!("echo: {method}")),
            "body was: {body}"
        );
    }

    client_handle.abort();
}

#[tokio::test]
async fn tunnel_request_with_no_subpath() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let (sink, stream) = connect_client(&base_url, "test-secret", "my-service").await;
    let client_handle = spawn_echo_client(sink, stream);

    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("{base_url}/tunnel/my-service"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "echo: GET /");

    client_handle.abort();
}

#[tokio::test]
async fn tunnel_request_with_query_params() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    // Custom client that echoes the query params
    let (mut sink, mut stream) = connect_client(&base_url, "test-secret", "my-service").await;

    let client_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                WsMessage::Text(t) => t,
                _ => continue,
            };

            match protocol::parse_message(&text) {
                Ok(Message::Request(req)) => {
                    let mut query_parts: Vec<String> =
                        req.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    query_parts.sort();
                    let resp = protocol::TunnelResponse {
                        msg_type: "response".to_string(),
                        request_id: req.request_id,
                        status: 200,
                        headers: HashMap::new(),
                        body: Some(format!("query: {}", query_parts.join("&"))),
                        body_encoding: None,
                    };
                    sink.send(WsMessage::text(protocol::serialize_response(&resp)))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = reqwest::get(format!(
        "{base_url}/tunnel/my-service/search?q=hello&page=1"
    ))
    .await
    .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("q=hello"), "body was: {body}");
    assert!(body.contains("page=1"), "body was: {body}");

    client_handle.abort();
}

#[tokio::test]
async fn tunnel_request_forwards_headers() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;
    let (mut sink, mut stream) = connect_client(&base_url, "test-secret", "my-service").await;

    // Client that echoes back the authorization header
    let client_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                WsMessage::Text(t) => t,
                _ => continue,
            };

            match protocol::parse_message(&text) {
                Ok(Message::Request(req)) => {
                    let auth = req
                        .headers
                        .get("authorization")
                        .cloned()
                        .unwrap_or_default();
                    let resp = protocol::TunnelResponse {
                        msg_type: "response".to_string(),
                        request_id: req.request_id,
                        status: 200,
                        headers: HashMap::new(),
                        body: Some(format!("auth: {auth}")),
                        body_encoding: None,
                    };
                    sink.send(WsMessage::text(protocol::serialize_response(&resp)))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{base_url}/tunnel/my-service/secure"))
        .header("Authorization", "Bearer my-token-123")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "auth: Bearer my-token-123");

    client_handle.abort();
}

#[tokio::test]
async fn returns_400_when_body_exceeds_max_size() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        max_body_size: 100, // 100 bytes
    });

    let base_url = start_server(server).await;

    let (sink, stream) = connect_client(&base_url, "test-secret", "my-service").await;
    let client_handle = spawn_echo_client(sink, stream);

    tokio::time::sleep(Duration::from_millis(50)).await;

    let http = reqwest::Client::new();
    let oversized_body = "x".repeat(200); // 200 bytes > 100 limit
    let resp = http
        .post(format!("{base_url}/tunnel/my-service/upload"))
        .body(oversized_body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);

    client_handle.abort();
}

#[tokio::test]
async fn client_sending_close_frame_disconnects_cleanly() {
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;

    let (mut sink, _stream) = connect_client(&base_url, "test-secret", "my-service").await;

    // Send a Close frame
    sink.send(WsMessage::Close(None)).await.unwrap();

    // Give the server time to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Requests should now fail with 502 since the client disconnected
    let resp = reqwest::get(format!("{base_url}/tunnel/my-service/test"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn client_sending_binary_frame_is_ignored() {
    // Binary WS frames should be silently ignored — only text matters.
    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(5),
        ..ServerConfig::default()
    });

    let base_url = start_server(server).await;
    let ws_url = base_url.replace("http://", "ws://") + "/ws";

    let (ws, _) = connect_async(&ws_url).await.unwrap();
    let (mut sink, mut stream) = ws.split();

    // Auth via text
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "test-secret".to_string(),
        service: "my-service".to_string(),
    };
    sink.send(WsMessage::text(serde_json::to_string(&auth).unwrap()))
        .await
        .unwrap();

    // Wait for auth_ok
    let resp = stream.next().await.unwrap().unwrap();
    assert!(matches!(resp, WsMessage::Text(_)));

    // Send a binary frame — should be ignored
    sink.send(WsMessage::Binary(vec![0xFF, 0xFE, 0xFD].into()))
        .await
        .unwrap();

    // Verify connection survived: send a text message after the binary frame
    sink.send(WsMessage::text(
        r#"{"type": "response", "request_id": "nonexistent", "status": 200}"#.to_string(),
    ))
    .await
    .unwrap();

    // No panic, no crash — binary frame was ignored
    sink.send(WsMessage::Close(None)).await.unwrap();
}
