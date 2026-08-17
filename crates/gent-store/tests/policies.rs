use gent_ports::{PolicyLedger, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{PermissionCategory, PermissionMode, PolicyRecord, PolicyScope, WorkspaceRecord};

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
        mode: PermissionMode::Default,
        allowed_tools: tools.iter().map(ToString::to_string).collect(),
        allowed_categories: Vec::new(),
    }
}

#[test]
fn permission_modes_and_category_approvals_are_durable() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    let mut record = policy(1, &[]);
    record.mode = PermissionMode::AutoAcceptEdits;
    record.allowed_categories = vec![PermissionCategory::Network];
    ledger.save_policy(&record).unwrap();
    assert_eq!(
        ledger
            .current_policy("workspace-a", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(record)
    );
}

#[test]
fn exact_policy_retries_return_without_appending_a_second_revision() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    let record = policy(1, &["git:status"]);
    ledger.save_policy(&record).unwrap();
    ledger.save_policy(&record).unwrap();
    assert_eq!(
        ledger
            .current_policy("workspace-a", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(record)
    );
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
