use serde_json::Value;
pub(super) fn read(call: &Value) -> Result<Option<(String, Value)>, &'static str> {
    fn valid(value: &str, max: usize) -> bool {
        !value.is_empty()
            && value.len() <= max
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    }
    let Some(name) = call["title"].as_str().filter(|v| v.starts_with("mcp__")) else {
        return Ok(None);
    };
    let (server, tool) = name
        .strip_prefix("mcp__")
        .and_then(|v| v.split_once("__"))
        .ok_or("invalid MCP tool binding")?;
    if !valid(server, 64) || !valid(tool, 128) {
        return Err("invalid MCP tool binding");
    }
    let args = call["rawInput"]
        .as_object()
        .ok_or("MCP arguments must be an object")?;
    let args = Value::Object(args.clone());
    if serde_json::to_vec(&args)
        .map_err(|_| "invalid MCP arguments")?
        .len()
        > 65536
    {
        return Err("MCP arguments exceed bound");
    }
    Ok(Some((name.into(), args)))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_name_and_input_ignore_read_hint() {
        let call = json!({"title":"mcp__gent_state__write","kind":"read","rawInput":{"path":"owned","value":3}});
        assert_eq!(
            read(&call).unwrap(),
            Some((
                "mcp__gent_state__write".into(),
                json!({"path":"owned","value":3})
            ))
        );
    }
    #[test]
    fn missing_scalar_and_oversize_args_do_not_fall_back_to_hints() {
        for args in [Value::Null, json!([]), json!({"large":"x".repeat(65536)})] {
            assert!(
                read(&json!({"title":"mcp__gent_state__write","kind":"read","rawInput":args}))
                    .is_err()
            );
        }
    }
    #[test]
    fn invalid_mcp_names_rejected() {
        for name in [
            "mcp__../x__write",
            "mcp____write",
            "mcp__server__",
            "mcp__server__bad/name",
        ] {
            assert!(read(&json!({"title":name,"rawInput":{}})).is_err());
        }
    }
    #[test]
    fn builtin_permission_protocol_unchanged() {
        assert_eq!(
            read(&json!({"title":"Approve Write","kind":"edit"})).unwrap(),
            None
        );
    }
}
