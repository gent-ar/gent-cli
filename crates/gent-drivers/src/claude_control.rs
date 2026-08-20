//! Strict Claude permission-control codec owned by the private process runner.

use serde_json::{Value, json};

/// The public identifiers needed to classify one Claude permission request.
///
/// Raw tool input and permission suggestions deliberately remain in the process runner. They are
/// neither durable facts nor client-facing values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudePermissionRequest {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
}

/// The closed response selected after Gent's durable permission policy resolves a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudePermissionBehavior {
    Allow,
    Deny,
}

/// Parses the documented `can_use_tool` control request and retains its suggestions for a later
/// response. No native input or suggestion data crosses this boundary.
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
    let mut response = json!({
        "behavior": match behavior {
            ClaudePermissionBehavior::Allow => "allow",
            ClaudePermissionBehavior::Deny => "deny",
        }
    });
    if behavior == ClaudePermissionBehavior::Allow && persist_suggestions && !suggestions.is_empty()
    {
        response["updatedPermissions"] = Value::Array(suggestions.to_vec());
    }
    let mut frame = serde_json::to_vec(&json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        }
    }))
    .expect("fixed Claude control response is serializable");
    frame.push(b'\n');
    frame
}

fn suggestions(value: &Value) -> Result<Vec<Value>, &'static str> {
    let values = value.as_array().ok_or("malformedClaudeControlRequest")?;
    values
        .iter()
        .all(Value::is_object)
        .then(|| values.to_vec())
        .ok_or("malformedClaudeControlRequest")
}

fn string<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name)?.as_str()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ClaudePermissionBehavior, encode_permission_response, parse_permission_request};

    #[test]
    fn permission_request_retains_only_identifiers_and_private_suggestions() {
        let (request, suggestions) = parse_permission_request(&json!({
            "type": "control_request",
            "request_id": "request-1",
            "request": {
                "subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "tool-1",
                "input": {"command": "private"},
                "permission_suggestions": [{"type": "addDirectories", "path": "/private"}],
            }
        }))
        .unwrap();
        assert_eq!(request.request_id, "request-1");
        assert_eq!(request.tool_use_id, "tool-1");
        assert_eq!(request.tool_name, "Bash");
        assert!(!format!("{request:?}").contains("private"));
        assert_eq!(suggestions.len(), 1);
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
