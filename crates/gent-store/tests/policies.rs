use gent_ports::{PolicyLedger, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{PolicyRecord, PolicyScope, WorkspaceRecord};

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        workspace_id: "workspace-a".into(),
        canonical_path: "/workspace".into(),
    }
}

fn policy(revision: u64, tools: &[&str]) -> PolicyRecord {
    PolicyRecord {
        policy_id: format!("policy-{revision}"),
        workspace_id: "workspace-a".into(),
        scope: PolicyScope::ProviderPermissions,
        revision,
        allowed_tools: tools.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn policy_revisions_are_durable_and_append_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    ledger.save_policy(&policy(1, &["git:status"])).unwrap();
    ledger
        .save_policy(&policy(2, &["git:status", "mcp/search"]))
        .unwrap();
    assert!(ledger.save_policy(&policy(2, &["git:status"])).is_err());
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted
            .current_policy("workspace-a", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(policy(2, &["git:status", "mcp/search"]))
    );
}

#[test]
fn policy_rejects_missing_workspace_and_noncanonical_allow_lists() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(ledger.save_policy(&policy(1, &["git:status"])).is_err());
    ledger.create_workspace(&workspace()).unwrap();
    assert!(
        ledger
            .save_policy(&policy(1, &["mcp/search", "git:status"]))
            .is_err()
    );
    assert!(ledger.save_policy(&policy(1, &["bad tool"])).is_err());
}
