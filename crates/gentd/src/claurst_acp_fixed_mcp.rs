use super::ClaurstAcpTransportError;
use serde_json::Value;

pub(crate) fn mcp_roster_sha256(servers: &[Value]) -> Result<String, ClaurstAcpTransportError> {
    use sha2::{Digest, Sha256};
    let servers: Vec<Value> = servers.iter().map(with_environment_map).collect();
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&servers).map_err(|_| ClaurstAcpTransportError::InvalidFrame)?
        )
    ))
}

fn with_environment_map(server: &Value) -> Value {
    let mut server = server.clone();
    let environment = server.get("env").and_then(Value::as_array).map(|entries| {
        entries
            .iter()
            .filter_map(|entry| {
                Some((
                    entry.get("name")?.as_str()?.to_owned(),
                    entry.get("value")?.clone(),
                ))
            })
            .collect::<serde_json::Map<_, _>>()
    });
    if let Some(environment) = environment {
        server["env"] = Value::Object(environment);
    }
    server
}

pub(super) fn matches(
    expected: Option<&str>,
    servers: &[Value],
) -> Result<bool, ClaurstAcpTransportError> {
    let Some(expected) = expected else {
        return Ok(false);
    };
    if mcp_roster_sha256(servers)? != expected {
        return Err(ClaurstAcpTransportError::InvalidSession);
    }
    Ok(true)
}
pub(super) fn check_sdk(response: &Value, required: bool) -> Result<(), ClaurstAcpTransportError> {
    if required
        && !response
            .pointer("/agentInfo/version")
            .and_then(Value::as_str)
            .is_some_and(|v| v.ends_with("+gent.mcp1"))
    {
        return Err(ClaurstAcpTransportError::HandshakeIncomplete);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn host_roster_digest_is_exact_before_frame() {
        let roster = vec![json!({"name":"one","args":["host-only"]})];
        let hash = mcp_roster_sha256(&roster).unwrap();
        assert_eq!(matches(Some(&hash), &roster), Ok(true));
        assert!(matches(Some(&hash), &[]).is_err());
        let changed = vec![json!({"name":"one","args":["other"]})];
        assert!(matches(Some(&hash), &changed).is_err());
    }
    #[test]
    fn settings_and_session_spellings_of_one_roster_have_one_digest() {
        let settings = vec![json!({"name":"one","command":"c","args":[],"env":{"K":"V"}})];
        let session =
            vec![json!({"name":"one","command":"c","args":[],"env":[{"name":"K","value":"V"}]})];
        let hash = mcp_roster_sha256(&settings).unwrap();
        assert_eq!(matches(Some(&hash), &session), Ok(true));
        let other =
            vec![json!({"name":"one","command":"c","args":[],"env":[{"name":"K","value":"W"}]})];
        assert!(matches(Some(&hash), &other).is_err());
    }
    #[test]
    fn legacy_without_host_binding_keeps_existing_protocol() {
        assert_eq!(matches(None, &[json!({"name":"legacy"})]), Ok(false));
        assert!(check_sdk(&json!({}), false).is_ok());
    }
    #[test]
    fn configured_mcp_requires_explicit_sdk_abi() {
        assert!(check_sdk(&json!({"agentInfo":{"version":"0.1.7"}}), true).is_err());
        assert!(check_sdk(&json!({"agentInfo":{"version":"0.1.7+gent.mcp1"}}), true).is_ok());
    }
}
