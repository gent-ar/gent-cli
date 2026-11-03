use super::*;

#[test]
#[ignore = "explicit read-only actual installed MCP contract measurement; no server execution"]
fn actual_installed_seven_server_contract_materializes_without_host_secrets() {
    let source =
        PathBuf::from(std::env::var_os("GENT_MCP_CONTRACT_SOURCE").expect("explicit host config"));
    let output =
        PathBuf::from(std::env::var_os("GENT_MCP_CONTRACT_OUTPUT").expect("owned output root"));
    let config = HostMcpConfig::capture(&source).unwrap();
    let names = [
        "gent-automations",
        "gent-browser",
        "gent-canvas",
        "gent-chat",
        "gent-forge",
        "gent-goal",
        "mate-canvas",
    ];
    assert_eq!(
        config.servers().keys().cloned().collect::<Vec<_>>(),
        names.map(str::to_string)
    );
    let worker = config.materialize(&output, &routes(&names)).unwrap();
    let bytes = std::fs::read(&worker.path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["mcpServers"].as_object().unwrap().len(), 7);
    for server in config.servers().values() {
        for secret in server.env.values().filter(|value| value.len() >= 16) {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
            );
        }
    }
    println!(
        "actual-contract servers=7 transformed-http=7 host-env-values-copied=0 sourceSha256={}",
        config.source_sha256()
    );
    drop(worker.handle);
}

fn document(root: &Path, names: &[&str]) -> serde_json::Value {
    let servers = names
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                serde_json::json!({
                    "command":root.join("node.exe"),"args":["host-private-script.js"],
                    "env":{"PRIVATE_TOKEN":"host-only-secret","PATH":"host-only-search-path"}
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::json!({"mcpServers":servers})
}
fn write(root: &Path, value: &serde_json::Value) -> PathBuf {
    let path = root.join("host.json");
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}
fn routes(names: &[&str]) -> BTreeMap<String, String> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| ((*name).into(), format!("{index:032x}")))
        .collect()
}

#[test]
fn preserves_all_seven_servers_without_copying_host_secrets_or_programs() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let names = [
        "gent-automations",
        "gent-browser",
        "gent-canvas",
        "gent-chat",
        "gent-forge",
        "gent-goal",
        "mate-canvas",
    ];
    let config = HostMcpConfig::capture(&write(&root, &document(&root, &names))).unwrap();
    assert_eq!(config.servers().len(), 7);
    assert_eq!(config.source_sha256().len(), 64);
    assert!(!format!("{config:?}").contains("host-only"));
    let worker = config.materialize(&root, &routes(&names)).unwrap();
    let bytes = std::fs::read(&worker.path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    for forbidden in [
        "host-only-secret",
        "host-only-search-path",
        "host-private-script.js",
        "command",
        "env",
    ] {
        assert!(!text.contains(forbidden));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["mcpServers"].as_object().unwrap().len(), 7);
    for name in names {
        let server = &value["mcpServers"][name];
        assert_eq!(server["type"], "http");
        assert!(
            server["url"]
                .as_str()
                .unwrap()
                .starts_with("${ANTHROPIC_BASE_URL}/__gent_mcp/")
        );
        assert_eq!(
            server["headers"]["Authorization"],
            "Bearer ${GENT_MCP_BRIDGE_TOKEN}"
        );
    }
    drop(worker.handle);
}

#[test]
fn never_drops_or_adds_a_server_to_cover_route_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let config = HostMcpConfig::capture(&write(&root, &document(&root, &["one", "two"]))).unwrap();
    for bad in [
        routes(&["one"]),
        routes(&["one", "other"]),
        routes(&["one", "two", "other"]),
    ] {
        assert!(config.materialize(&root, &bad).is_err());
    }
}

#[test]
fn rejects_arbitrary_duplicate_or_unbounded_route_ids() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let config = HostMcpConfig::capture(&write(&root, &document(&root, &["one", "two"]))).unwrap();
    for bad in [
        "http://other/",
        "../escape",
        &"A".repeat(32),
        &"a".repeat(33),
    ] {
        let mut map = routes(&["one", "two"]);
        map.insert("one".into(), bad.into());
        assert!(config.materialize(&root, &map).is_err());
    }
    let mut map = routes(&["one", "two"]);
    map.insert("two".into(), "0".repeat(32));
    assert!(config.materialize(&root, &map).is_err());
}

#[test]
fn rejects_duplicate_json_fields_and_unsupported_transport_without_secret_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let path = root.join("invalid.json");
    for raw in [
        r#"{"mcpServers":{},"mcpServers":{}}"#,
        r#"{"mcpServers":{"one":{},"one":{}}}"#,
        r#"{"mcpServers":{},"host-only-secret":true}"#,
    ] {
        std::fs::write(&path, raw).unwrap();
        let error = HostMcpConfig::capture(&path).unwrap_err();
        assert!(!error.contains("host-only-secret"));
    }
    for transport in ["http", "sse", "unexpected"] {
        let mut value = document(&root, &["one"]);
        value["mcpServers"]["one"]["type"] = transport.into();
        assert!(HostMcpConfig::capture(&write(&root, &value)).is_err());
    }
}

#[test]
fn rejects_relative_program_nul_arguments_and_excessive_server_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let mut value = document(&root, &["one"]);
    value["mcpServers"]["one"]["command"] = "node".into();
    assert!(HostMcpConfig::capture(&write(&root, &value)).is_err());
    let mut value = document(&root, &["one"]);
    value["mcpServers"]["one"]["args"] = serde_json::json!(["x\u{0}y"]);
    assert!(HostMcpConfig::capture(&write(&root, &value)).is_err());
    let names = (0..33).map(|i| format!("server{i}")).collect::<Vec<_>>();
    let refs = names.iter().map(String::as_str).collect::<Vec<_>>();
    assert!(HostMcpConfig::capture(&write(&root, &document(&root, &refs))).is_err());
}

#[test]
fn size_bound_and_empty_configuration_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let path = root.join("oversized.json");
    std::fs::write(&path, vec![b' '; usize::try_from(MAX_BYTES).unwrap() + 1]).unwrap();
    assert!(HostMcpConfig::capture(&path).is_err());
    let config = HostMcpConfig::capture(&write(&root, &document(&root, &[]))).unwrap();
    let worker = config.materialize(&root, &BTreeMap::new()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&worker.path).unwrap()).unwrap(),
        serde_json::json!({"mcpServers":{}})
    );
}

#[cfg(windows)]
#[test]
fn published_worker_file_is_immutable_while_owner_retains_read_handle() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let config = HostMcpConfig::capture(&write(&root, &document(&root, &["one"]))).unwrap();
    let worker = config.materialize(&root, &routes(&["one"])).unwrap();
    assert!(std::fs::write(&worker.path, b"changed").is_err());
    assert!(std::fs::remove_file(&worker.path).is_err());
    drop(worker.handle);
}
