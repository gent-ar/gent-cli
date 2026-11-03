use super::StandaloneMcpConfig;

#[test]
fn loads_only_bounded_stdio_servers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp.json");
    std::fs::write(&path, r#"{"mcpServers":{"example":{"command":"node","args":["server.js"],"env":{"TOKEN":"secret"}}}}"#).unwrap();
    let config = StandaloneMcpConfig::load(&path).unwrap();
    assert_eq!(
        config.codex_servers().unwrap()["example"]["command"],
        "node"
    );
    assert_eq!(config.claurst_servers().unwrap()[0]["name"], "example");
    assert_eq!(
        config.claurst_servers().unwrap()[0]["env"],
        serde_json::json!([{"name":"TOKEN","value":"secret"}])
    );
    assert_eq!(
        config.claurst_settings_servers().unwrap()[0]["env"],
        serde_json::json!({"TOKEN":"secret"})
    );
    assert_eq!(
        config.claurst_servers().unwrap()[0]["args"],
        serde_json::json!(["server.js"])
    );
    let before = config.digest().unwrap();
    std::fs::write(
        &path,
        r#"{"mcpServers":{"example":{"command":"node","args":["updated.js"]}}}"#,
    )
    .unwrap();
    assert_ne!(before, config.digest().unwrap());
    assert_eq!(
        config.claurst_servers().unwrap()[0]["args"],
        serde_json::json!(["updated.js"])
    );
    assert_eq!(
        config.claurst_servers().unwrap()[0]["env"],
        serde_json::json!([])
    );
}
