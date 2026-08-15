use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, PolicyRecord, PolicyScope, WorkspaceRecord};

#[test]
fn coordinator_exposes_only_current_secret_free_policy_revision() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-a".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    let policy = PolicyRecord {
        policy_id: "policy-1".into(),
        workspace_id: "workspace-a".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        allowed_tools: vec!["git:status".into()],
    };
    coordinator.save_policy(&policy).unwrap();
    assert_eq!(
        coordinator
            .current_policy("workspace-a", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(policy)
    );
}
