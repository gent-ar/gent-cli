//! Strict Claude permission-control codec owned by the private process runner.

use serde_json::{Value, json};

const MAX_UPDATED_INPUT_BYTES: usize = 16 * 1024;
// Leave room under the native 1MiB write-frame bound for control metadata and
// bounded permission suggestions. This is private retained input, never a preview.
pub(crate) const MAX_PRESERVED_INPUT_BYTES: usize = 512 * 1024;
pub(crate) const MAX_PERMISSION_REPLY_BYTES: usize = 1024 * 1024;
pub(crate) fn valid_explicit_updated_input(input: &Value) -> bool {
    input.is_object()
        && serde_json::to_vec(input).is_ok_and(|bytes| bytes.len() <= MAX_UPDATED_INPUT_BYTES)
}

/// The public identifiers needed to classify one Claude permission request.
///
/// Raw tool input and permission suggestions remain in the process runner. Only a
/// bounded, redacted operation preview crosses the client-facing boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudePermissionRequest {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub operation_preview: Option<Value>,
    pub child_id: Option<String>,
}

/// The closed response selected after Gent's durable permission policy resolves a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudePermissionBehavior {
    Allow,
    Deny,
}

/// Parses the documented `can_use_tool` control request and retains its suggestions for a later
/// response. Only the shared presentation preview of input crosses this boundary.
pub fn parse_permission_request(
    frame: &Value,
) -> Result<(ClaudePermissionRequest, Vec<Value>), &'static str> {
    let request = frame
        .get("request")
        .ok_or("malformedClaudeControlRequest")?;
    if string(frame, "type") != Some("control_request")
        || string(request, "subtype") != Some("can_use_tool")
    {
        return Err("unsupportedClaudeControlRequest");
    }
    let request_id = string(frame, "request_id")
        .or_else(|| string(request, "request_id"))
        .filter(|value| !value.is_empty())
        .ok_or("malformedClaudeControlRequest")?;
    let tool_use_id = string(request, "tool_use_id")
        .filter(|value| !value.is_empty())
        .ok_or("malformedClaudeControlRequest")?;
    let tool_name = string(request, "tool_name")
        .filter(|value| !value.is_empty())
        .ok_or("malformedClaudeControlRequest")?;
    let suggestions = request
        .get("permission_suggestions")
        .or_else(|| request.get("suggestions"))
        .map_or(Ok(Vec::new()), suggestions)?;
    Ok((
        ClaudePermissionRequest {
            request_id: request_id.into(),
            tool_use_id: tool_use_id.into(),
            tool_name: tool_name.into(),
            operation_preview: request.get("input").and_then(gent_types::operation_preview),
            child_id: string(request, "agent_id")
                .or_else(|| string(frame, "agent_id"))
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        },
        suggestions,
    ))
}

/// Encodes the only supported Claude permission response.
///
/// Suggestions are echoed only for an allowed persistent decision, matching Claude's documented
/// `updatedPermissions` relay. A denial never returns provider-supplied suggestion data.
#[must_use]
pub fn encode_permission_response(
    request_id: &str,
    behavior: ClaudePermissionBehavior,
    persist_suggestions: bool,
    suggestions: &[Value],
) -> Vec<u8> {
    encode_permission_response_with_input(
        request_id,
        behavior,
        persist_suggestions,
        suggestions,
        None,
    )
}

pub const PLAN_REVIEW_MESSAGE: &str = "Gent saved this plan for the user's review. End your turn now without further tool calls; the user will approve it and start implementation separately.";

#[must_use]
pub fn encode_plan_review_response(request_id: &str) -> Vec<u8> {
    let Ok(mut frame) = serde_json::to_vec(&json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {"behavior": "deny", "message": PLAN_REVIEW_MESSAGE},
        }
    })) else {
        return Vec::new();
    };
    frame.push(b'\n');
    frame
}

pub fn encode_permission_response_with_input(
    request_id: &str,
    behavior: ClaudePermissionBehavior,
    persist_suggestions: bool,
    suggestions: &[Value],
    updated_input: Option<&Value>,
) -> Vec<u8> {
    encode_permission_response_bounded(
        request_id,
        behavior,
        persist_suggestions,
        suggestions,
        updated_input,
        MAX_UPDATED_INPUT_BYTES,
    )
}

pub(crate) fn encode_permission_response_preserving_input(
    request_id: &str,
    behavior: ClaudePermissionBehavior,
    persist_suggestions: bool,
    suggestions: &[Value],
    input: &Value,
) -> Option<Vec<u8>> {
    if !input.is_object()
        || !serde_json::to_vec(input).is_ok_and(|bytes| bytes.len() <= MAX_PRESERVED_INPUT_BYTES)
    {
        return None;
    }
    let bytes = encode_permission_response_bounded(
        request_id,
        behavior,
        persist_suggestions,
        suggestions,
        Some(input),
        MAX_PRESERVED_INPUT_BYTES,
    );
    (!bytes.is_empty() && bytes.len() <= MAX_PERMISSION_REPLY_BYTES).then_some(bytes)
}

fn encode_permission_response_bounded(
    request_id: &str,
    behavior: ClaudePermissionBehavior,
    persist_suggestions: bool,
    suggestions: &[Value],
    updated_input: Option<&Value>,
    max_input_bytes: usize,
) -> Vec<u8> {
    let mut response = json!({
        "behavior": match behavior {
            ClaudePermissionBehavior::Allow => "allow",
            ClaudePermissionBehavior::Deny => "deny",
        }
    });
    if behavior == ClaudePermissionBehavior::Deny {
        response["message"] = "User denied the request".into();
    }
    if behavior == ClaudePermissionBehavior::Allow && persist_suggestions && !suggestions.is_empty()
    {
        response["updatedPermissions"] = Value::Array(suggestions.to_vec());
    }
    if behavior == ClaudePermissionBehavior::Allow {
        if let Some(updated_input) =
            updated_input
                .filter(|value| value.is_object())
                .filter(|value| {
                    serde_json::to_vec(value).is_ok_and(|encoded| encoded.len() <= max_input_bytes)
                })
        {
            response["updatedInput"] = updated_input.clone();
        }
    }
    let Ok(mut frame) = serde_json::to_vec(&json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        }
    })) else {
        return Vec::new();
    };
    frame.push(b'\n');
    frame
}

#[must_use]
pub fn control_request_id(frame: &Value) -> Option<&str> {
    string(frame, "request_id")
        .or_else(|| {
            frame
                .get("request")
                .and_then(|request| string(request, "request_id"))
        })
        .filter(|value| !value.is_empty() && value.len() <= 256)
}

#[must_use]
pub fn encode_control_error(request_id: &str, classification: &str) -> Vec<u8> {
    let Ok(mut frame) = serde_json::to_vec(&json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": format!("Gent does not handle this control request ({classification})."),
        }
    })) else {
        return Vec::new();
    };
    frame.push(b'\n');
    frame
}

fn suggestions(value: &Value) -> Result<Vec<Value>, &'static str> {
    let values = value.as_array().ok_or("malformedClaudeControlRequest")?;
    values
        .iter()
        .all(Value::is_object)
        .then(|| values.clone())
        .ok_or("malformedClaudeControlRequest")
}

fn string<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name)?.as_str()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ClaudePermissionBehavior, encode_permission_response,
        encode_permission_response_with_input, parse_permission_request,
    };

    #[test]
    fn native_windows_powershell_permission_preserves_the_actual_command_schema() {
        for command in [
            "powershell.exe -NoProfile -Command Write-Output",
            "powershell.exe -NoProfile -Command Write-Output GENT-PARITY-COMMAND-OK",
        ] {
            let (request, _) = parse_permission_request(&json!({
                "type":"control_request", "request_id":"permission-powershell",
                "request":{"subtype":"can_use_tool", "tool_name":"PowerShell", "tool_use_id":"tool-powershell",
                    "input":{"command":command,"description":"Print a verification marker from a child PowerShell"}}
            })).unwrap();
            let preview = request.operation_preview.unwrap();
            assert_eq!(preview["command"], command);
            assert_eq!(
                preview["description"],
                "Print a verification marker from a child PowerShell"
            );
        }
    }

    #[test]
    fn permission_request_exposes_operation_details_but_keeps_suggestions_private() {
        let (request, suggestions) = parse_permission_request(&json!({
            "type": "control_request",
            "request_id": "request-1",
            "request": {
                "subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "tool-1",
                "input": {"command": "cargo test", "environment":{"TOKEN":"private"}},
                "permission_suggestions": [{"type": "addDirectories", "path": "/private"}],
            }
        }))
        .unwrap();
        assert_eq!(request.request_id, "request-1");
        assert_eq!(request.tool_use_id, "tool-1");
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(
            request.operation_preview,
            Some(json!({"command":"cargo test"}))
        );
        assert!(!format!("{request:?}").contains("private"));
        assert_eq!(suggestions.len(), 1);
        assert_eq!(request.child_id, None);
    }

    #[test]
    fn a_subagent_permission_request_carries_the_asking_child_agent() {
        let (request, _) = parse_permission_request(&json!({
            "type": "control_request",
            "request_id": "request-2",
            "request": {
                "subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "tool-2",
                "agent_id": "agent-7",
            }
        }))
        .unwrap();
        assert_eq!(request.child_id.as_deref(), Some("agent-7"));
    }

    #[test]
    fn allow_can_echo_suggestions_but_deny_never_does() {
        let suggestions = vec![json!({"type": "addDirectories", "path": "/private"})];
        let allowed: serde_json::Value = serde_json::from_slice(&encode_permission_response(
            "request-1",
            ClaudePermissionBehavior::Allow,
            true,
            &suggestions,
        ))
        .unwrap();
        assert_eq!(
            allowed["response"]["response"]["updatedPermissions"],
            serde_json::Value::Array(suggestions.clone())
        );
        let denied: serde_json::Value = serde_json::from_slice(&encode_permission_response(
            "request-1",
            ClaudePermissionBehavior::Deny,
            true,
            &suggestions,
        ))
        .unwrap();
        assert!(
            denied["response"]["response"]
                .get("updatedPermissions")
                .is_none()
        );
        assert_eq!(
            denied["response"]["response"]["message"],
            "User denied the request"
        );
    }

    #[test]
    fn allowed_updated_input_is_relayed_but_denied_input_is_dropped() {
        let input = json!({"plan":"approved", "scope":"turn"});
        let allowed: serde_json::Value =
            serde_json::from_slice(&encode_permission_response_with_input(
                "request-1",
                ClaudePermissionBehavior::Allow,
                false,
                &[],
                Some(&input),
            ))
            .unwrap();
        assert_eq!(allowed["response"]["response"]["updatedInput"], input);

        let denied: serde_json::Value =
            serde_json::from_slice(&encode_permission_response_with_input(
                "request-1",
                ClaudePermissionBehavior::Deny,
                false,
                &[],
                Some(&input),
            ))
            .unwrap();
        assert!(denied["response"]["response"].get("updatedInput").is_none());
    }

    #[test]
    fn oversized_updated_input_is_not_written() {
        let input = json!({"value":"x".repeat(17 * 1024)});
        let encoded: serde_json::Value =
            serde_json::from_slice(&encode_permission_response_with_input(
                "request-1",
                ClaudePermissionBehavior::Allow,
                false,
                &[],
                Some(&input),
            ))
            .unwrap();
        assert!(
            encoded["response"]["response"]
                .get("updatedInput")
                .is_none()
        );
    }

    #[test]
    fn malformed_or_unknown_control_requests_fail_closed() {
        for frame in [
            json!({"type": "control_request", "request": {"subtype": "can_use_tool"}}),
            json!({"type": "control_request", "request": {"subtype": "future"}}),
            json!({"type": "control_request", "request": {"subtype": "can_use_tool", "request_id": "a", "tool_use_id": "b", "tool_name": "Bash", "permission_suggestions": ["not an object"]}}),
        ] {
            assert!(parse_permission_request(&frame).is_err());
        }
    }
}
