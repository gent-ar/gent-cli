//! Explicit tools-only protocol negotiation. Reverse RPC is not advertised.
use serde_json::{Value, json};
pub fn forwarded(message: &Value) -> Result<Value, super::mcp::Error> {
    let mut value = message.clone();
    if value["method"] == "initialize" {
        value
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .ok_or(super::mcp::Error::Protocol)?
            .insert("capabilities".into(), json!({}));
    }
    if let Some(meta) = value
        .get_mut("params")
        .and_then(|v| v.get_mut("_meta"))
        .and_then(Value::as_object_mut)
    {
        meta.remove("progressToken");
    }
    Ok(value)
}
pub fn response(method: &str, mut value: Value) -> Result<Value, super::mcp::Error> {
    if method == "initialize" && value.get("result").is_some() {
        let result = value
            .get_mut("result")
            .and_then(Value::as_object_mut)
            .ok_or(super::mcp::Error::Protocol)?;
        let tools = result
            .get("capabilities")
            .and_then(|v| v.get("tools"))
            .is_some();
        result.insert(
            "capabilities".into(),
            if tools {
                json!({"tools":{"listChanged":false}})
            } else {
                json!({})
            },
        );
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_subset_does_not_advertise_reverse_rpc_or_progress() {
        let original = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"sampling":{},"roots":{},"elicitation":{}}}});
        assert_eq!(
            forwarded(&original).unwrap()["params"]["capabilities"],
            json!({})
        );
        assert!(original["params"]["capabilities"].get("sampling").is_some());
        let command = json!({"method":"tools/call","params":{"_meta":{"progressToken":"token"},"name":"read","arguments":{"path":"safe"}}});
        let forwarded = forwarded(&command).unwrap();
        assert!(forwarded["params"]["_meta"].get("progressToken").is_none());
        assert_eq!(
            forwarded["params"]["arguments"],
            command["params"]["arguments"]
        );
    }
    #[test]
    fn mcp_subset_server_capabilities_are_an_explicit_tools_only_contract() {
        let r=response("initialize",json!({"id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true},"resources":{"subscribe":true},"prompts":{},"logging":{}}}})).unwrap();
        assert_eq!(
            r["result"]["capabilities"],
            json!({"tools":{"listChanged":false}})
        );
    }
}
