use super::*;
use axum::http::HeaderValue;
use outpunch::protocol::TunnelResponse;

// --- parse_query ---

#[test]
fn parse_query_none() {
    assert_eq!(parse_query(None), HashMap::new());
}

#[test]
fn parse_query_empty_string() {
    assert_eq!(parse_query(Some("")), HashMap::new());
}

#[test]
fn parse_query_single_param() {
    let result = parse_query(Some("foo=bar"));
    assert_eq!(result.get("foo").unwrap(), "bar");
    assert_eq!(result.len(), 1);
}

#[test]
fn parse_query_multiple_params() {
    let result = parse_query(Some("foo=bar&baz=qux&num=42"));
    assert_eq!(result.get("foo").unwrap(), "bar");
    assert_eq!(result.get("baz").unwrap(), "qux");
    assert_eq!(result.get("num").unwrap(), "42");
    assert_eq!(result.len(), 3);
}

#[test]
fn parse_query_ignores_malformed_pairs() {
    let result = parse_query(Some("foo=bar&bad&baz=qux"));
    assert_eq!(result.get("foo").unwrap(), "bar");
    assert_eq!(result.get("baz").unwrap(), "qux");
    assert_eq!(result.len(), 2);
}

#[test]
fn parse_query_value_with_equals() {
    // only splits on first =
    let result = parse_query(Some("expr=a=b"));
    assert_eq!(result.get("expr").unwrap(), "a=b");
}

// --- extract_headers ---

#[test]
fn extract_headers_passes_through_normal_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("authorization", HeaderValue::from_static("Bearer token"));

    let result = extract_headers(&headers);
    assert_eq!(result.get("content-type").unwrap(), "application/json");
    assert_eq!(result.get("authorization").unwrap(), "Bearer token");
}

#[test]
fn extract_headers_strips_hop_by_hop() {
    let mut headers = HeaderMap::new();
    headers.insert("host", HeaderValue::from_static("example.com"));
    headers.insert("connection", HeaderValue::from_static("keep-alive"));
    headers.insert("upgrade", HeaderValue::from_static("websocket"));
    headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
    headers.insert("x-custom", HeaderValue::from_static("keep-me"));

    let result = extract_headers(&headers);
    assert!(!result.contains_key("host"));
    assert!(!result.contains_key("connection"));
    assert!(!result.contains_key("upgrade"));
    assert!(!result.contains_key("transfer-encoding"));
    assert_eq!(result.get("x-custom").unwrap(), "keep-me");
    assert_eq!(result.len(), 1);
}

#[test]
fn extract_headers_empty() {
    let headers = HeaderMap::new();
    let result = extract_headers(&headers);
    assert!(result.is_empty());
}

// --- tunnel_response_to_axum ---

#[tokio::test]
async fn response_plain_body() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 200,
        headers: HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
        body: Some("hello world".to_string()),
        body_encoding: None,
    };

    let axum_resp = tunnel_response_to_axum(resp);
    assert_eq!(axum_resp.status(), StatusCode::OK);
    assert_eq!(
        axum_resp.headers().get("content-type").unwrap(),
        "text/plain"
    );

    let body = axum::body::to_bytes(axum_resp.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"hello world");
}

#[tokio::test]
async fn response_base64_body() {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let original = "hello from tunnel";
    let encoded = BASE64.encode(original);

    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 200,
        headers: HashMap::new(),
        body: Some(encoded),
        body_encoding: Some("base64".to_string()),
    };

    let axum_resp = tunnel_response_to_axum(resp);
    let body = axum::body::to_bytes(axum_resp.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"hello from tunnel");
}

#[tokio::test]
async fn response_invalid_base64_falls_back_to_raw() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 200,
        headers: HashMap::new(),
        body: Some("not valid base64!!!".to_string()),
        body_encoding: Some("base64".to_string()),
    };

    let axum_resp = tunnel_response_to_axum(resp);
    let body = axum::body::to_bytes(axum_resp.into_body(), 1024)
        .await
        .unwrap();
    // Falls back to raw bytes of the string
    assert_eq!(body.as_ref(), b"not valid base64!!!");
}

#[tokio::test]
async fn response_empty_body() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 204,
        headers: HashMap::new(),
        body: None,
        body_encoding: None,
    };

    let axum_resp = tunnel_response_to_axum(resp);
    assert_eq!(axum_resp.status(), StatusCode::NO_CONTENT);

    let body = axum::body::to_bytes(axum_resp.into_body(), 1024)
        .await
        .unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn response_preserves_status_codes() {
    for (code, expected) in [
        (200, StatusCode::OK),
        (201, StatusCode::CREATED),
        (400, StatusCode::BAD_REQUEST),
        (404, StatusCode::NOT_FOUND),
        (500, StatusCode::INTERNAL_SERVER_ERROR),
        (502, StatusCode::BAD_GATEWAY),
        (504, StatusCode::GATEWAY_TIMEOUT),
    ] {
        let resp = TunnelResponse {
            msg_type: "response".to_string(),
            request_id: "abc".to_string(),
            status: code,
            headers: HashMap::new(),
            body: None,
            body_encoding: None,
        };

        let axum_resp = tunnel_response_to_axum(resp);
        assert_eq!(axum_resp.status(), expected, "failed for status {code}");
    }
}

#[tokio::test]
async fn response_multiple_headers() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 200,
        headers: HashMap::from([
            ("content-type".to_string(), "application/pdf".to_string()),
            (
                "content-disposition".to_string(),
                "attachment; filename=\"report.pdf\"".to_string(),
            ),
            ("x-custom".to_string(), "value".to_string()),
        ]),
        body: None,
        body_encoding: None,
    };

    let axum_resp = tunnel_response_to_axum(resp);
    assert_eq!(
        axum_resp.headers().get("content-type").unwrap(),
        "application/pdf"
    );
    assert_eq!(
        axum_resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"report.pdf\""
    );
    assert_eq!(axum_resp.headers().get("x-custom").unwrap(), "value");
}

#[tokio::test]
async fn response_with_invalid_header_name_returns_500() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc".to_string(),
        status: 200,
        headers: HashMap::from([("invalid header\n".to_string(), "value".to_string())]),
        body: Some("ok".to_string()),
        body_encoding: None,
    };

    let axum_resp = tunnel_response_to_axum(resp);
    // Invalid header name causes builder to fail, falls back to 500
    assert_eq!(axum_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn slow_reading_client_does_not_lose_requests() {
    use outpunch::server::ServerConfig;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    const REQUESTS: usize = 200;

    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout: Duration::from_secs(10),
        ..ServerConfig::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(server)).await.unwrap();
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(ClientMessage::text(
        r#"{"type":"auth","token":"test-secret","service":"my-service"}"#,
    ))
    .await
    .unwrap();
    let _auth_ok = ws.next().await.unwrap().unwrap();

    let http = reqwest::Client::new();
    let body = "x".repeat(256 * 1024);
    let mut requests = Vec::new();
    for _ in 0..REQUESTS {
        let http = http.clone();
        let body = body.clone();
        requests.push(tokio::spawn(async move {
            http.post(format!("http://{addr}/tunnel/my-service/upload"))
                .body(body)
                .send()
                .await
        }));
    }

    // Client stops reading long enough for the socket buffers to fill.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut received = 0;
    while let Ok(Some(Ok(ClientMessage::Text(text)))) =
        tokio::time::timeout(Duration::from_secs(2), ws.next()).await
    {
        if text.contains(r#""type":"request""#) {
            received += 1;
        }
    }

    assert_eq!(
        received, REQUESTS,
        "requests were dropped before reaching the client"
    );

    for r in requests {
        r.abort();
    }
}

// F5 in spec/FINDINGS.md (spec/AxumLate.cfg): requests whose callers already got 504 are
// dropped from the adapter queue instead of written. The client keeps reading, slowly: each
// write finishes well within the request timeout (so F7's write timeout never fires), but
// requests queued behind the others time out before their turn.
#[tokio::test]
async fn timed_out_requests_are_not_delivered_later() {
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    const REQUESTS: usize = 200;

    let addr = serve(Duration::from_millis(500)).await;
    let mut ws = authed_client(addr).await;

    let reader = tokio::spawn(async move {
        // Waits longer for the first frame, so a slow start cannot end the reader early.
        let mut received = 0;
        let mut quiet = Duration::from_secs(10);
        while let Ok(Some(Ok(message))) = tokio::time::timeout(quiet, ws.next()).await {
            quiet = Duration::from_secs(2);
            if let ClientMessage::Text(text) = message
                && text.contains(r#""type":"request""#)
            {
                received += 1;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        received
    });

    // 504, or 502 for a request still in flight when the reader gives up and disconnects.
    assert!(
        upload_all(addr, REQUESTS)
            .await
            .iter()
            .all(|&s| s == 504 || s == 502)
    );
    let received = reader.await.unwrap();

    // Requests written before their timeout still arrive: about 20 in 500 ms at this pace, plus
    // what the socket buffers hold. Without the check, all 200 arrive.
    assert!(received > 0, "no request reached the client at all");
    assert!(
        received <= REQUESTS / 4,
        "{received} of {REQUESTS} requests reached the client after their callers got 504"
    );
}

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(timeout: Duration) -> std::net::SocketAddr {
    use outpunch::server::ServerConfig;

    let server = OutpunchServer::new(ServerConfig {
        secret: "test-secret".to_string(),
        timeout,
        ..ServerConfig::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(server)).await.unwrap();
    });
    addr
}

async fn authed_client(addr: std::net::SocketAddr) -> ClientSocket {
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(ClientMessage::text(
        r#"{"type":"auth","token":"test-secret","service":"my-service"}"#,
    ))
    .await
    .unwrap();
    let _auth_ok = ws.next().await.unwrap().unwrap();
    ws
}

/// Sends `n` requests with 256 KB bodies and returns their status codes.
async fn upload_all(addr: std::net::SocketAddr, n: usize) -> Vec<u16> {
    let http = reqwest::Client::new();
    let body = "x".repeat(256 * 1024);
    let requests: Vec<_> = (0..n)
        .map(|_| {
            let http = http.clone();
            let body = body.clone();
            tokio::spawn(async move {
                http.post(format!("http://{addr}/tunnel/my-service/upload"))
                    .body(body)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            })
        })
        .collect();
    let mut statuses = Vec::new();
    for r in requests {
        statuses.push(r.await.unwrap());
    }
    statuses
}

// F7 in spec/FINDINGS.md (spec/AxumStuck.cfg): a client that stays connected but stops reading
// is disconnected once one write has waited longer than the request timeout, so its service is
// unregistered and new requests fail fast instead of queueing.
#[tokio::test]
async fn client_that_stops_reading_is_disconnected() {
    let addr = serve(Duration::from_millis(500)).await;
    let _ws = authed_client(addr).await;

    // The client never reads: 50 MB fills the socket buffers and blocks a write. Requests time
    // out (504) until the write timeout closes the connection; those still in flight then fail
    // with 502.
    assert!(
        upload_all(addr, 200)
            .await
            .iter()
            .all(|&s| s == 504 || s == 502)
    );
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let started = std::time::Instant::now();
    assert_eq!(upload_all(addr, 1).await, vec![502]);
    assert!(started.elapsed() < Duration::from_millis(400));
}

// F6 in spec/FINDINGS.md (spec/AxumStalled.cfg): once a connection is over, a write task
// blocked on a client that never reads is aborted and the socket closed, without waiting for
// the request timeout (F7). Seen from the client: its sends start failing (the server's socket
// is gone, so its kernel resets the connection).
#[tokio::test]
async fn socket_of_a_finished_connection_closes_even_if_the_client_never_reads() {
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    let addr = serve(Duration::from_secs(30)).await;
    let mut stalled = authed_client(addr).await;

    // Fill the socket so the write task blocks, then end the connection by replacing it.
    let uploads = tokio::spawn(upload_all(addr, 100));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let _newer = authed_client(addr).await;
    assert!(uploads.await.unwrap().iter().all(|&s| s == 502));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut failed = false;
    while tokio::time::Instant::now() < deadline {
        let sent = tokio::time::timeout(
            Duration::from_millis(200),
            stalled.send(ClientMessage::text("x".repeat(1024))),
        )
        .await;
        if matches!(sent, Ok(Err(_))) {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        failed,
        "the server kept the finished connection's socket open"
    );
}
