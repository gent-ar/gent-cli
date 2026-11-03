//! Whole-set structural admission; this never hashes, spawns or grants authority.
use super::mcp::Error;
use crate::contained_claude_mcp::HostMcpConfig;
use gent_types::RunVersionLock;
use std::{collections::BTreeMap, path::Path};

pub(super) fn validate(
    config: &HostMcpConfig,
    programs: &BTreeMap<String, RunVersionLock>,
    entries: &BTreeMap<String, RunVersionLock>,
) -> Result<(), Error> {
    if config.servers().keys().ne(programs.keys()) {
        return Err(Error::Denied);
    }
    let expected: Vec<_> = programs
        .iter()
        .filter(|(_, lock)| lock.provider == "node-mcp")
        .map(|(name, _)| name)
        .collect();
    if entries.keys().ne(expected) {
        return Err(Error::Denied);
    }
    for (name, server) in config.servers() {
        let lock = programs.get(name).ok_or(Error::Denied)?;
        if lock.provider == "node-mcp" {
            let role =
                gent_linux_sandbox::node_profile::Role::from_server(name).ok_or(Error::Denied)?;
            let entry = entries.get(name).ok_or(Error::Denied)?;
            if lock.provider != "node-mcp"
                || lock.canonical_path != server.command
                || server.args.len() != 1
                || server.args[0] != entry.canonical_path
                || entry.provider != "node-mcp-entry"
                || entry.compatibility_entry != format!("gent-node-bundle:{name}:v1")
                || gent_linux_sandbox::node_profile::selected_environment(
                    role,
                    &server.env,
                    "/private/structural-check",
                )
                .is_none()
            {
                return Err(Error::Denied);
            }
            continue;
        }
        if lock.provider != "gent-mcp"
            || lock.canonical_path != server.command
            || !gent_linux_sandbox::host_stdio::validate_profile(
                Path::new(&server.command),
                &server.args,
                &server.env,
            )
        {
            return Err(Error::Denied);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source(value: serde_json::Value) -> (tempfile::TempDir, HostMcpConfig) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"mcpServers":value})).unwrap(),
        )
        .unwrap();
        let config = HostMcpConfig::capture(&path).unwrap();
        (root, config)
    }
    fn server() -> serde_json::Value {
        json!({"command":"/opt/gent/gent","args":["--no-autostart","--data-dir","/home/qa/data","mcp","automations"],"env":{"PATH":"/usr/bin:/bin","HOME":"/home/qa"}})
    }
    fn lock() -> RunVersionLock {
        RunVersionLock {
            provider: "gent-mcp".into(),
            canonical_path: "/opt/gent/gent".into(),
            file_identity: "not-used-by-structural-test".into(),
            digest_sha256: "0".repeat(64),
            version: "fixture".into(),
            compatibility_entry: "structural-only".into(),
        }
    }
    #[test]
    fn native_roster_preserves_every_selected_name() {
        let (_r, c) = source(json!({"a":server(),"b":server()}));
        let mut locks = BTreeMap::from([("a".into(), lock()), ("b".into(), lock())]);
        assert!(validate(&c, &locks, &BTreeMap::new()).is_ok());
        locks.remove("b");
        assert!(validate(&c, &locks, &BTreeMap::new()).is_err());
        locks.insert("b".into(), lock());
        locks.insert("unselected".into(), lock());
        assert!(validate(&c, &locks, &BTreeMap::new()).is_err());
    }
    #[test]
    fn native_roster_unsupported_member_rejects_entire_set() {
        for name in ["a", "b", "z"] {
            let mut servers = json!({"a":server(),"b":server(),"z":server()});
            servers[name]["command"] = json!("/opt/node/node");
            servers[name]["args"] = json!(["/opt/server/main.js"]);
            let (_r, c) = source(servers);
            let mut locks = BTreeMap::from([
                ("a".into(), lock()),
                ("b".into(), lock()),
                ("z".into(), lock()),
            ]);
            locks.get_mut(name).unwrap().canonical_path = "/opt/node/node".into();
            assert!(validate(&c, &locks, &BTreeMap::new()).is_err());
        }
    }
    #[test]
    fn native_roster_rejects_loader_environment_and_identity_substitution() {
        let mut s = server();
        s["env"]["LD_PRELOAD"] = json!("/workspace/evil.so");
        let (_r, c) = source(json!({"a":s}));
        assert!(
            validate(
                &c,
                &BTreeMap::from([("a".into(), lock())]),
                &BTreeMap::new()
            )
            .is_err()
        );
        let (_r, c) = source(json!({"a":server()}));
        let mut substituted = lock();
        substituted.canonical_path = "/other/gent".into();
        assert!(
            validate(
                &c,
                &BTreeMap::from([("a".into(), substituted)]),
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}
