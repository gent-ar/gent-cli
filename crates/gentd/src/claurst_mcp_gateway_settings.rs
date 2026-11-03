use super::EgressError;
pub(super) fn settings(
    contents: &str,
    clients: Option<serde_json::Value>,
) -> Result<serde_json::Value, EgressError> {
    let mut settings: serde_json::Value =
        serde_json::from_str(contents).map_err(|_| EgressError::Configuration)?;
    let object = settings.as_object_mut().ok_or(EgressError::Configuration)?;
    let config = object
        .get_mut("config")
        .and_then(|value| value.as_object_mut())
        .ok_or(EgressError::Configuration)?;
    if config.get("provider").and_then(|value| value.as_str()) != Some("llama-cpp") {
        return Err(EgressError::Configuration);
    }
    let original = config
        .get("mcp_servers")
        .and_then(|v| v.as_array())
        .ok_or(EgressError::Configuration)?;
    if let Some(clients) = clients {
        validate_clients(original, &clients)?;
        config.insert("mcp_servers".into(), clients);
    } else if !original.is_empty() {
        return Err(EgressError::Configuration);
    }
    // Claurst 0.1.7's local adapter omits Authorization. Its supported
    // custom-openai adapter authenticates the same local-model protocol.
    // Only the helper projects the public port; the bearer remains in ENV.
    let model = config
        .get("model")
        .and_then(|v| v.as_str())
        .and_then(|m| m.strip_prefix("llama-cpp/"))
        .filter(|m| !m.is_empty())
        .ok_or(EgressError::Configuration)?;
    let model = format!("custom-openai/{model}");
    config.insert("model".into(), serde_json::json!(model));
    config.insert("provider".into(), serde_json::json!("custom-openai"));
    config.insert("provider_configs".into(), serde_json::json!({}));
    config.insert(
        "api_key".into(),
        serde_json::json!("{env:CUSTOM_OPENAI_API_KEY}"),
    );
    object.insert("provider".into(), serde_json::json!("custom-openai"));
    object.insert(
        "providers".into(),
        serde_json::json!({"custom-openai": {
            "enabled": true, "api_base": "http://127.0.0.1:0/__gent_claurst_proxy_v1__/v1"
        }}),
    );
    Ok(settings)
}

fn validate_clients(
    original: &[serde_json::Value],
    clients: &serde_json::Value,
) -> Result<(), EgressError> {
    use std::collections::BTreeSet;
    fn name(v: &serde_json::Value) -> Result<&str, EgressError> {
        v.as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            .ok_or(EgressError::Configuration)
    }
    let entries = clients
        .as_array()
        .filter(|v| v.len() <= 32)
        .ok_or(EgressError::Configuration)?;
    let mut expected = BTreeSet::new();
    for entry in original {
        if !expected.insert(name(&entry["name"])?) {
            return Err(EgressError::Configuration);
        }
    }
    let mut names = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut routes = BTreeSet::new();
    for entry in entries {
        if entry.as_object().is_none_or(|v| {
            v.len() != 6
                || v.keys().any(|k| {
                    !["name", "type", "command", "args", "url", "env"].contains(&k.as_str())
                })
        }) || entry["type"] != "http"
            || !entry["command"].is_null()
            || entry["args"] != serde_json::json!([])
            || entry["env"] != serde_json::json!({"MCP_AUTH_TOKEN":"${GENT_MCP_BRIDGE_TOKEN}"})
        {
            return Err(EgressError::Configuration);
        }
        let key = name(&entry["name"])?;
        if !names.insert(key) || !aliases.insert(key.replace('-', "_")) {
            return Err(EgressError::Configuration);
        }
        let route = entry["url"]
            .as_str()
            .and_then(|v| v.strip_prefix("${GENT_MCP_BRIDGE_ORIGIN}/__gent_mcp/"))
            .filter(|v| {
                v.len() == 32
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
            })
            .ok_or(EgressError::Configuration)?;
        if !routes.insert(route) {
            return Err(EgressError::Configuration);
        }
    }
    if names != expected {
        return Err(EgressError::Configuration);
    }
    Ok(())
}

#[cfg(test)]
#[path = "claurst_mcp_gateway_settings_tests.rs"]
mod tests;
