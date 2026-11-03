use super::*;
use serde_json::json;
use std::path::PathBuf;
fn source(root: &Path, names: &[&str]) -> PathBuf {
    let entries:Vec<_>=names.iter().map(|name|json!({"name":name,"type":"stdio","command":root.join("gent.exe"),"args":["host-private-script"],"env":{"PRIVATE_TOKEN":"public-fixture-host-secret"}})).collect();
    let path = root.join("settings.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"provider":"llama-cpp","config":{"model":"public-model","mcp_servers":entries}})).unwrap()).unwrap();
    path
}
fn routes(names: &[&str]) -> BTreeMap<String, String> {
    names
        .iter()
        .enumerate()
        .map(|(n, name)| ((*name).into(), format!("{n:032x}")))
        .collect()
}
#[test]
fn claurst_config_is_host_locked_and_worker_document_contains_only_route_refs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = source(&root, &["gent-state", "gent-chat"]);
    let before = std::fs::read(&path).unwrap();
    let config = HostMcpConfig::capture_claurst(&path).unwrap();
    assert_eq!(config.source_sha256(), hex::encode(Sha256::digest(&before)));
    #[cfg(windows)]
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    let worker = config
        .materialize_claurst(&root, &routes(&["gent-state", "gent-chat"]))
        .unwrap();
    let bytes = std::fs::read(&worker.path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let entries = value["mcpServers"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert_eq!(entry["type"], "http");
        assert!(entry["command"].is_null());
        assert_eq!(entry["args"], json!([]));
        assert!(
            entry["url"]
                .as_str()
                .unwrap()
                .starts_with("${GENT_MCP_BRIDGE_ORIGIN}/__gent_mcp/")
        );
        assert_eq!(
            entry["env"],
            json!({"MCP_AUTH_TOKEN":"${GENT_MCP_BRIDGE_TOKEN}"})
        );
    }
    assert!(!text.contains("host-private-script") && !text.contains("public-fixture-host-secret"));
    assert_eq!(std::fs::read(path).unwrap(), before);
    drop(worker);
}
#[test]
fn claurst_duplicate_and_alias_colliding_names_fail_closed() {
    for names in [vec!["same", "same"], vec!["a-b", "a_b"]] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        assert!(HostMcpConfig::capture_claurst(&source(&root, &names)).is_err());
    }
}
#[test]
fn claude_alias_collision_also_rejected_before_peer_start() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("host.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"mcpServers":{"a-b":{"command":root.join("gent.exe")},"a_b":{"command":root.join("gent.exe")}}})).unwrap()).unwrap();
    assert!(HostMcpConfig::capture(&path).is_err());
}
#[test]
fn claurst_http_unknown_fields_relative_commands_and_oversize_sets_rejected() {
    for variant in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = source(&root, &["one"]);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let entry = &mut value["config"]["mcp_servers"][0];
        match variant {
            0 => entry["type"] = json!("http"),
            1 => entry["headers"] = json!({"Authorization":"forbidden"}),
            2 => entry["command"] = json!("relative.exe"),
            _ => {
                let mut entries = vec![];
                for n in 0..33 {
                    let mut entry = entry.clone();
                    entry["name"] = json!(format!("server{n}"));
                    entries.push(entry);
                }
                value["config"]["mcp_servers"] = json!(entries);
            }
        }
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(HostMcpConfig::capture_claurst(&path).is_err());
    }
}
#[test]
fn claurst_route_coverage_is_exact_before_worker_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let config = HostMcpConfig::capture_claurst(&source(&root, &["one", "two"])).unwrap();
    for bad in [
        routes(&["one"]),
        routes(&["one", "other"]),
        routes(&["one", "two", "other"]),
    ] {
        assert!(config.materialize_claurst(&root, &bad).is_err());
    }
}
#[test]
fn duplicate_config_and_env_keys_are_not_silently_last_wins() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("duplicate.json");
    for value in [
        r#"{"config":{"mcp_servers":[]},"config":{"mcp_servers":[]}}"#,
        r#"{"config":{"mcp_servers":[{"name":"one","command":"x","env":{"TOKEN":"one","TOKEN":"two"}}]}}"#,
    ] {
        std::fs::write(&path, value).unwrap();
        assert!(HostMcpConfig::capture_claurst(&path).is_err());
    }
}
