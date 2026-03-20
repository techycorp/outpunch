use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What the adapter passes to the core. No request_id — the core generates that.
#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub service: String,
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

/// Wire protocol: request sent from server to client over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TunnelRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub service: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

/// Wire protocol: response sent from client to server over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TunnelResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub status: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_encoding: Option<String>,
}

/// Wire protocol: auth message sent from client to server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub token: String,
    pub service: String,
}

/// Wire protocol: auth success response from server to client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthOk {
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Wire protocol: auth failure response from server to client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthError {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub message: String,
}

/// Parsed message from the WebSocket.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Auth(AuthMessage),
    AuthOk(AuthOk),
    AuthError(AuthError),
    Request(TunnelRequest),
    Response(TunnelResponse),
}

/// Parse a raw JSON string into a Message.
pub fn parse_message(raw: &str) -> Result<Message, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;

    let msg_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or("missing 'type' field")?;

    match msg_type {
        "auth" => {
            let msg: AuthMessage =
                serde_json::from_value(value).map_err(|e| format!("invalid auth message: {e}"))?;
            Ok(Message::Auth(msg))
        }
        "auth_ok" => {
            let msg: AuthOk = serde_json::from_value(value)
                .map_err(|e| format!("invalid auth_ok message: {e}"))?;
            Ok(Message::AuthOk(msg))
        }
        "auth_error" => {
            let msg: AuthError = serde_json::from_value(value)
                .map_err(|e| format!("invalid auth_error message: {e}"))?;
            Ok(Message::AuthError(msg))
        }
        "request" => {
            let msg: TunnelRequest = serde_json::from_value(value)
                .map_err(|e| format!("invalid request message: {e}"))?;
            Ok(Message::Request(msg))
        }
        "response" => {
            let msg: TunnelResponse = serde_json::from_value(value)
                .map_err(|e| format!("invalid response message: {e}"))?;
            Ok(Message::Response(msg))
        }
        other => Err(format!("unknown message type: {other}")),
    }
}

/// Serialize a TunnelRequest to JSON string.
pub fn serialize_request(req: &TunnelRequest) -> String {
    serde_json::to_string(req).expect("TunnelRequest serialization should never fail")
}

/// Serialize a TunnelResponse to JSON string.
pub fn serialize_response(resp: &TunnelResponse) -> String {
    serde_json::to_string(resp).expect("TunnelResponse serialization should never fail")
}

/// Build a TunnelRequest from an IncomingRequest, generating a request_id.
pub fn build_tunnel_request(incoming: &IncomingRequest) -> TunnelRequest {
    TunnelRequest {
        msg_type: "request".to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        service: incoming.service.clone(),
        method: incoming.method.clone(),
        path: incoming.path.clone(),
        query: incoming.query.clone(),
        headers: incoming.headers.clone(),
        body: incoming.body.clone(),
    }
}

/// Build an error TunnelResponse.
pub fn error_response(request_id: &str, status: u16, message: &str) -> TunnelResponse {
    TunnelResponse {
        msg_type: "response".to_string(),
        request_id: request_id.to_string(),
        status,
        headers: HashMap::from([("Content-Type".to_string(), "application/json".to_string())]),
        body: Some(format!("{{\"error\":\"{message}\"}}")),
        body_encoding: None,
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
