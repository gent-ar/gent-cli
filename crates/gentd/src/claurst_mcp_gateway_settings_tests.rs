use super::*;
use serde_json::json;
fn source(names: &[&str]) -> String {
    let entries:Vec<_>=names.iter().map(|name|json!({"name":name,"type":"stdio","command":"host-private-program","args":["host-private-argument"],"env":{"SECRET":"public-fixture-secret"}})).collect();
    json!({"config":{"provider":"llama-cpp","model":"llama-cpp/public-model","mcp_servers":entries,"api_key":"old-private-api","provider_configs":{"old":"private"}},"providers":{"old":"private"}}).to_string()
}
fn clients(names: &[&str]) -> serde_json::Value {
    json!(names.iter().enumerate().map(|(n,name)|json!({"name":name,"type":"http","command":null,"args":[],"url":format!("${{GENT_MCP_BRIDGE_ORIGIN}}/__gent_mcp/{n:032x}"),"env":{"MCP_AUTH_TOKEN":"${GENT_MCP_BRIDGE_TOKEN}"}})).collect::<Vec<_>>())
}
#[test]
fn client_refs_replace_host_config_without_carrying_host_secrets() {
    let output = settings(&source(&["one", "two"]), Some(clients(&["one", "two"]))).unwrap();
    let text = output.to_string();
    for secret in ["host-private", "public-fixture-secret", "old-private-api"] {
        assert!(!text.contains(secret));
    }
    assert_eq!(output["config"]["mcp_servers"], clients(&["one", "two"]));
    assert_eq!(output["config"]["model"], "custom-openai/public-model");
    assert_eq!(output["config"]["api_key"], "{env:CUSTOM_OPENAI_API_KEY}");
}
#[test]
fn missing_owner_cannot_silently_drop_configured_mcp() {
    assert!(settings(&source(&["one"]), None).is_err());
    assert!(settings(&source(&[]), None).is_ok());
}
#[test]
fn exact_roster_and_route_ids_required() {
    for names in [
        vec!["one"],
        vec!["other", "two"],
        vec!["one", "two", "extra"],
    ] {
        assert!(settings(&source(&["one", "two"]), Some(clients(&names))).is_err());
    }
    let mut value = clients(&["one", "two"]);
    value[1]["url"] = value[0]["url"].clone();
    assert!(settings(&source(&["one", "two"]), Some(value)).is_err());
}
#[test]
fn literal_credentials_host_urls_missing_keys_and_extra_fields_rejected() {
    for variant in 0..6 {
        let mut value = clients(&["one"]);
        let row = &mut value[0];
        match variant {
            0 => row["url"] = json!("http://127.0.0.1:1/evil"),
            1 => row["env"]["MCP_AUTH_TOKEN"] = json!("literal-token"),
            2 => row["headers"] = json!({"Authorization":"forbidden"}),
            3 => {
                row.as_object_mut().unwrap().remove("command");
                row["secret"] = json!("extra");
            }
            4 => row["args"] = json!(["host-argument"]),
            _ => row["type"] = json!("stdio"),
        }
        assert!(settings(&source(&["one"]), Some(value)).is_err());
    }
}
#[test]
fn model_adapter_validation_is_preserved() {
    for bad in [
        "[]",
        "{}",
        r#"{"config":{"provider":"other","model":"llama-cpp/x","mcp_servers":[]}}"#,
        r#"{"config":{"provider":"llama-cpp","model":"remote/x","mcp_servers":[]}}"#,
    ] {
        assert!(settings(bad, None).is_err());
    }
}
