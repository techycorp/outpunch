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
