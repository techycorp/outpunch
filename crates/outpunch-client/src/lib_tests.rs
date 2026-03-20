use std::collections::HashMap;

use axum::Router;
use axum::routing::get;
use outpunch::protocol::TunnelRequest;
use tokio::net::TcpListener;

use super::*;

// --- forward_url ---

#[test]
fn forward_url_simple_path() {
    let req = make_request("api/test", HashMap::new());
    assert_eq!(
        forward_url("http://localhost:8080", &req),
        "http://localhost:8080/api/test"
    );
}

#[test]
fn forward_url_strips_trailing_slash() {
    let req = make_request("api/test", HashMap::new());
    assert_eq!(
        forward_url("http://localhost:8080/", &req),
        "http://localhost:8080/api/test"
    );
}

#[test]
fn forward_url_with_query_params() {
    let query = HashMap::from([("foo".to_string(), "bar".to_string())]);
    let req = make_request("search", query);
    let url = forward_url("http://localhost:8080", &req);
    assert!(url.starts_with("http://localhost:8080/search?"));
    assert!(url.contains("foo=bar"));
}

#[test]
fn forward_url_with_multiple_query_params() {
    let query = HashMap::from([
        ("a".to_string(), "1".to_string()),
        ("b".to_string(), "2".to_string()),
    ]);
    let req = make_request("path", query);
    let url = forward_url("http://localhost:8080", &req);
    assert!(url.contains("a=1"));
    assert!(url.contains("b=2"));
    assert!(url.contains("&"));
}

#[test]
fn forward_url_empty_path() {
    let req = make_request("", HashMap::new());
    assert_eq!(
        forward_url("http://localhost:8080", &req),
        "http://localhost:8080/"
    );
}

// --- forward_request ---

async fn start_local_server(handler: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, handler).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn forward_request_get_returns_response() {
    let app = Router::new().route("/api/test", get(|| async { "hello from local" }));
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };
    let req = make_request("api/test", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body_encoding.as_deref(), Some("base64"));

    // Decode base64 body
    let decoded = BASE64.decode(resp.body.as_ref().unwrap()).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "hello from local");
}

#[tokio::test]
async fn forward_request_preserves_request_id() {
    let app = Router::new().route("/test", get(|| async { "ok" }));
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };
    let req = make_request("test", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.request_id, req.request_id);
    assert_eq!(resp.msg_type, "response");
}

#[tokio::test]
async fn forward_request_returns_502_when_service_unreachable() {
    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: "http://127.0.0.1:1".to_string(), // nothing listening
        ..ClientConfig::default()
    };
    let req = make_request("test", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 502);
    assert!(resp.body.as_ref().unwrap().contains("error"));
}

#[tokio::test]
async fn forward_request_returns_504_on_timeout() {
    let app = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "too slow"
        }),
    );
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        request_timeout: Duration::from_millis(50),
        ..ClientConfig::default()
    };
    let req = make_request("slow", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 504);
    assert!(resp.body.as_ref().unwrap().contains("timeout"));
}

#[tokio::test]
async fn forward_request_post_with_body() {
    use axum::body::Bytes;

    let app = Router::new().route(
        "/echo",
        axum::routing::post(|body: Bytes| async move { body }),
    );
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };

    let mut req = make_request("echo", HashMap::new());
    req.method = "POST".to_string();
    req.body = Some("request body content".to_string());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 200);
    let decoded = BASE64.decode(resp.body.as_ref().unwrap()).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "request body content");
}

#[tokio::test]
async fn forward_request_forwards_headers() {
    use axum::http::HeaderMap;

    let app = Router::new().route(
        "/headers",
        get(|headers: HeaderMap| async move {
            headers
                .get("x-custom")
                .map(|v| v.to_str().unwrap().to_string())
                .unwrap_or_else(|| "missing".to_string())
        }),
    );
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };

    let mut req = make_request("headers", HashMap::new());
    req.headers
        .insert("x-custom".to_string(), "my-value".to_string());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 200);
    let decoded = BASE64.decode(resp.body.as_ref().unwrap()).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "my-value");
}

#[tokio::test]
async fn forward_request_captures_response_headers() {
    use axum::response::IntoResponse;

    let app = Router::new().route(
        "/with-headers",
        get(|| async { ([("x-response-header", "response-value")], "body").into_response() }),
    );
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };
    let req = make_request("with-headers", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.headers.get("x-response-header").unwrap(),
        "response-value"
    );
}

#[tokio::test]
async fn forward_request_handles_404() {
    let app = Router::new(); // no routes — everything 404s
    let base_url = start_local_server(app).await;

    let client = reqwest::Client::new();
    let config = ClientConfig {
        forward_to: base_url,
        ..ClientConfig::default()
    };
    let req = make_request("nonexistent", HashMap::new());

    let resp = forward_request(&client, &config, &req).await;

    assert_eq!(resp.status, 404);
}

// --- run_connection integration tests ---

use outpunch::server::{OutpunchServer, ServerConfig as CoreServerConfig};

async fn start_outpunch_server(secret: &str) -> String {
    let server = OutpunchServer::new(CoreServerConfig {
        secret: secret.to_string(),
        timeout: std::time::Duration::from_secs(5),
        ..CoreServerConfig::default()
    });
    let app = outpunch_axum::router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

#[tokio::test]
async fn run_connection_authenticates_and_receives_requests() {
    let ws_url = start_outpunch_server("test-secret").await;
    let local_app = Router::new().route("/api/hello", get(|| async { "world" }));
    let local_url = start_local_server(local_app).await;

    let config = ClientConfig {
        server_url: ws_url.clone(),
        secret: "test-secret".to_string(),
        service: "my-service".to_string(),
        forward_to: local_url,
        ..ClientConfig::default()
    };

    // Run connection in background — it will block listening for requests
    let config_clone = config.clone();
    let conn_handle = tokio::spawn(async move { run_connection(&config_clone).await });

    // Give client time to connect and auth
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Make an HTTP request through the tunnel
    let http_url = ws_url.replace("ws://", "http://").replace("/ws", "");
    let resp = reqwest::get(format!("{http_url}/tunnel/my-service/api/hello"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // The response body is base64 encoded by the client, decoded by the adapter
    let body = resp.text().await.unwrap();
    assert_eq!(body, "world");

    conn_handle.abort();
}

#[tokio::test]
async fn run_connection_fails_with_wrong_secret() {
    let ws_url = start_outpunch_server("correct-secret").await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "wrong-secret".to_string(),
        service: "my-service".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("auth rejected"));
}

#[tokio::test]
async fn run_connection_fails_when_server_unreachable() {
    let config = ClientConfig {
        server_url: "ws://127.0.0.1:1/ws".to_string(), // nothing listening
        secret: "secret".to_string(),
        service: "my-service".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_err());
}

// --- edge case tests using a raw WS server for precise control ---

use axum::extract::ws::{Message as AxumWsMsg, WebSocket, WebSocketUpgrade};

/// Start a WS server that runs a custom handler for each connection.
async fn start_raw_ws_server<F, Fut>(handler: F) -> String
where
    F: Fn(WebSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let app = Router::new().route(
        "/ws",
        axum::routing::get(move |ws: WebSocketUpgrade| {
            let handler = handler.clone();
            async move { ws.on_upgrade(move |socket| handler(socket)) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://127.0.0.1:{}/ws", addr.port())
}

#[tokio::test]
async fn run_connection_handles_server_close_frame() {
    // Server authenticates then immediately sends Close
    let ws_url = start_raw_ws_server(|mut socket| async move {
        // Read auth
        let _ = socket.recv().await;
        // Send auth_ok
        socket
            .send(AxumWsMsg::Text(r#"{"type":"auth_ok"}"#.into()))
            .await
            .unwrap();
        // Send Close
        socket.send(AxumWsMsg::Close(None)).await.unwrap();
    })
    .await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "s".to_string(),
        service: "svc".to_string(),
        ..ClientConfig::default()
    };

    // Should return Ok — clean close
    let result = run_connection(&config).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn run_connection_ignores_non_request_messages() {
    // Server sends auth_ok, then garbage text, then closes
    let ws_url = start_raw_ws_server(|mut socket| async move {
        let _ = socket.recv().await;
        socket
            .send(AxumWsMsg::Text(r#"{"type":"auth_ok"}"#.into()))
            .await
            .unwrap();
        // Send non-request messages
        socket
            .send(AxumWsMsg::Text("not json".into()))
            .await
            .unwrap();
        socket
            .send(AxumWsMsg::Text(r#"{"type":"auth_ok"}"#.into()))
            .await
            .unwrap();
        // Close
        socket.send(AxumWsMsg::Close(None)).await.unwrap();
    })
    .await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "s".to_string(),
        service: "svc".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn run_connection_ignores_binary_frames() {
    // Server sends auth_ok, then a binary frame, then closes
    let ws_url = start_raw_ws_server(|mut socket| async move {
        let _ = socket.recv().await;
        socket
            .send(AxumWsMsg::Text(r#"{"type":"auth_ok"}"#.into()))
            .await
            .unwrap();
        socket
            .send(AxumWsMsg::Binary(vec![0xFF, 0xFE].into()))
            .await
            .unwrap();
        socket.send(AxumWsMsg::Close(None)).await.unwrap();
    })
    .await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "s".to_string(),
        service: "svc".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn authenticate_fails_on_binary_auth_response() {
    // Server sends binary frame instead of auth_ok text
    let ws_url = start_raw_ws_server(|mut socket| async move {
        let _ = socket.recv().await;
        socket
            .send(AxumWsMsg::Binary(vec![0x00].into()))
            .await
            .unwrap();
    })
    .await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "s".to_string(),
        service: "svc".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unexpected WS message type")
    );
}

#[tokio::test]
async fn authenticate_fails_on_unexpected_message_type() {
    // Server sends a request message instead of auth_ok
    let ws_url = start_raw_ws_server(|mut socket| async move {
        let _ = socket.recv().await;
        socket
            .send(AxumWsMsg::Text(
                r#"{"type":"request","request_id":"x","service":"s","method":"GET","path":"/"}"#
                    .into(),
            ))
            .await
            .unwrap();
    })
    .await;

    let config = ClientConfig {
        server_url: ws_url,
        secret: "s".to_string(),
        service: "svc".to_string(),
        ..ClientConfig::default()
    };

    let result = run_connection(&config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unexpected message during auth")
    );
}

fn make_request(path: &str, query: HashMap<String, String>) -> TunnelRequest {
    TunnelRequest {
        msg_type: "request".to_string(),
        request_id: "test-req-id".to_string(),
        service: "my-service".to_string(),
        method: "GET".to_string(),
        path: path.to_string(),
        query,
        headers: HashMap::new(),
        body: None,
    }
}
