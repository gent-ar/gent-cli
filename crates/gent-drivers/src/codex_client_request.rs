//! Bounded responses to documented Codex app-server client requests.
//!
//! These requests are process-private control traffic. The turn driver writes a response
//! immediately and never turns their provider payloads into durable public facts.

use serde_json::{Value, json};

/// Outcome of classifying one inbound Codex app-server request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexClientRequestResponse {
    /// The request is not a client request owned by this boundary.
    NotHandled,
    /// A recognized request was malformed and cannot safely be answered.
    Malformed,
    /// Write this newline-delimited JSON-RPC response to the owned process.
    Write(Vec<u8>),
}

/// Encodes the native app's fail-closed responses for documented server-to-client requests.
///
/// `now_epoch_seconds` is injected so the protocol encoder remains deterministic. A request ID
/// is echoed only when it is a non-empty string or a non-negative integer JSON-RPC identifier.
#[must_use]
pub fn respond_to_codex_client_request(
    frame: &Value,
    now_epoch_seconds: u64,
) -> CodexClientRequestResponse {
    let Some(method) = frame.get("method").and_then(Value::as_str) else {
        return CodexClientRequestResponse::NotHandled;
    };
    if !matches!(
        method,
        "item/tool/call"
            | "account/chatgptAuthTokens/refresh"
            | "attestation/generate"
            | "currentTime/read"
    ) {
        return CodexClientRequestResponse::NotHandled;
    }
    let Some(id) = request_id(frame) else {
        return CodexClientRequestResponse::Malformed;
    };
    let response = match method {
        "item/tool/call" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "success": false,
                "contentItems": [{
                    "type": "inputText",
                    "text": "Gent has no registered executor for this experimental Codex dynamic tool."
                }]
            }
        }),
        "account/chatgptAuthTokens/refresh" => refusal(
            &id,
            "Gent does not manage Codex ChatGPT auth tokens; use Codex login/config for this account.",
        ),
        "attestation/generate" => refusal(
            &id,
            "Gent did not opt in to Codex client attestation and cannot generate attestation tokens.",
        ),
        "currentTime/read" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"currentTimeAt": now_epoch_seconds}
        }),
        _ => return CodexClientRequestResponse::NotHandled,
    };
    let Ok(mut encoded) = serde_json::to_vec(&response) else {
        return CodexClientRequestResponse::Malformed;
    };
    encoded.push(b'\n');
    CodexClientRequestResponse::Write(encoded)
}

fn request_id(frame: &Value) -> Option<Value> {
    match frame.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Some(Value::String(id.clone())),
        Some(Value::Number(id)) if id.as_u64().is_some() => Some(Value::Number(id.clone())),
        _ => None,
    }
}

fn refusal(id: &Value, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32603, "message": message}
    })
}
