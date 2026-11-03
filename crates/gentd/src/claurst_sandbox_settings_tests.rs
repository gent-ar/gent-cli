use super::*;
use std::collections::BTreeMap;

fn launch(settings: &serde_json::Value) -> (tempfile::TempDir, LocalProcessLaunch) {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("settings.json"), settings.to_string()).unwrap();
    let launch = LocalProcessLaunch {
        executable: "claurst".into(),
        arguments: vec!["acp".into()],
        environment: BTreeMap::from([("CLAURST_HOME".into(), home.path().display().to_string())]),
        working_directory: None,
    };
    (home, launch)
}

#[test]
fn a_configured_roster_is_fixed_for_host_and_contained_claurst_alike() {
    let server = serde_json::json!({"name": "gent-chat", "command": "gent", "args": ["mcp", "chat"], "env": {}});
    let (_home, configured) = launch(&serde_json::json!({"config": {"mcp_servers": [server]}}));
    let (_, _, _, roster) = isolated_settings(&configured).unwrap();
    assert_eq!(
        roster,
        Some(crate::claurst_acp_transport::mcp_roster_sha256(&[server]).unwrap())
    );
    let (_home, empty) = launch(&serde_json::json!({"config": {"mcp_servers": []}}));
    assert_eq!(isolated_settings(&empty).unwrap().3, None);
}

#[test]
fn host_shells_are_only_offered_when_the_packaged_runtime_has_them() {
    for (name, path) in packaged_shells() {
        assert!(matches!(
            name.as_str(),
            "GENT_GIT_BASH_PATH" | "GENT_POWERSHELL_PATH"
        ));
        assert!(std::path::Path::new(&path).is_file());
    }
}
