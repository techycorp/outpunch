use super::*;

#[test]
fn serialize_and_parse_tunnel_request() {
    let req = TunnelRequest {
        msg_type: "request".to_string(),
        request_id: "abc-123".to_string(),
        service: "my-service".to_string(),
        method: "POST".to_string(),
        path: "api/test".to_string(),
        query: HashMap::from([("foo".to_string(), "bar".to_string())]),
        headers: HashMap::from([("Content-Type".to_string(), "application/json".to_string())]),
        body: Some("{\"data\":1}".to_string()),
    };

    let json = serialize_request(&req);
    let parsed = parse_message(&json).unwrap();

    assert_eq!(parsed, Message::Request(req));
}

#[test]
fn serialize_and_parse_tunnel_response() {
    let resp = TunnelResponse {
        msg_type: "response".to_string(),
        request_id: "abc-123".to_string(),
        status: 200,
        headers: HashMap::from([("Content-Type".to_string(), "application/json".to_string())]),
        body: Some("ok".to_string()),
        body_encoding: None,
    };

    let json = serialize_response(&resp);
    let parsed = parse_message(&json).unwrap();

    assert_eq!(parsed, Message::Response(resp));
}

#[test]
fn serialize_and_parse_auth_message() {
    let auth = AuthMessage {
        msg_type: "auth".to_string(),
        token: "secret".to_string(),
        service: "my-service".to_string(),
    };

    let json = serde_json::to_string(&auth).unwrap();
    let parsed = parse_message(&json).unwrap();

    assert_eq!(parsed, Message::Auth(auth));
}

#[test]
fn serialize_and_parse_auth_ok() {
    let auth_ok = AuthOk {
        msg_type: "auth_ok".to_string(),
    };

    let json = serde_json::to_string(&auth_ok).unwrap();
    let parsed = parse_message(&json).unwrap();

    assert_eq!(parsed, Message::AuthOk(auth_ok));
}

#[test]
fn serialize_and_parse_auth_error() {
    let auth_err = AuthError {
        msg_type: "auth_error".to_string(),
        message: "invalid token".to_string(),
    };

    let json = serde_json::to_string(&auth_err).unwrap();
    let parsed = parse_message(&json).unwrap();

    assert_eq!(parsed, Message::AuthError(auth_err));
}

#[test]
fn parse_invalid_json() {
    let result = parse_message("not json");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid JSON"));
}

#[test]
fn parse_missing_type_field() {
    let result = parse_message(r#"{"foo": "bar"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("missing 'type' field"));
}

#[test]
fn parse_unknown_type() {
    let result = parse_message(r#"{"type": "unknown"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown message type"));
}

#[test]
fn parse_request_with_missing_fields() {
    let result = parse_message(r#"{"type": "request"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid request message"));
}

#[test]
fn parse_response_with_defaults() {
    let json = r#"{"type": "response", "request_id": "abc", "status": 200}"#;
    let parsed = parse_message(json).unwrap();

    if let Message::Response(resp) = parsed {
        assert_eq!(resp.request_id, "abc");
        assert_eq!(resp.status, 200);
        assert!(resp.headers.is_empty());
        assert!(resp.body.is_none());
        assert!(resp.body_encoding.is_none());
    } else {
        panic!("expected Response");
    }
}

#[test]
fn build_tunnel_request_generates_unique_ids() {
    let incoming = IncomingRequest {
        service: "my-service".to_string(),
        method: "GET".to_string(),
        path: "test".to_string(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: None,
    };

    let req1 = build_tunnel_request(&incoming);
    let req2 = build_tunnel_request(&incoming);

    assert_ne!(req1.request_id, req2.request_id);
    assert_eq!(req1.msg_type, "request");
    assert_eq!(req1.service, "my-service");
}

#[test]
fn error_response_builds_correctly() {
    let resp = error_response("abc-123", 502, "tunnel offline");

    assert_eq!(resp.request_id, "abc-123");
    assert_eq!(resp.status, 502);
    assert_eq!(resp.msg_type, "response");
    assert!(resp.body.as_ref().unwrap().contains("tunnel offline"));
}

#[test]
fn response_with_base64_body_encoding() {
    let json = r#"{
        "type": "response",
        "request_id": "abc",
        "status": 200,
        "body": "SGVsbG8gV29ybGQ=",
        "body_encoding": "base64"
    }"#;

    let parsed = parse_message(json).unwrap();

    if let Message::Response(resp) = parsed {
        assert_eq!(resp.body_encoding.as_deref(), Some("base64"));
        assert_eq!(resp.body.as_deref(), Some("SGVsbG8gV29ybGQ="));
    } else {
        panic!("expected Response");
    }
}
