use gent_ports::PolicyLedger;
use gent_types::PolicyRecord;

pub(crate) fn policy_for<L>(
    ledger: &L,
    workspace_id: &str,
) -> Result<PolicyRecord, gent_ports::LedgerError>
where
    L: PolicyLedger,
{
    ledger.ensure_default_provider_permission_policy(workspace_id)
}

#[cfg(test)]
mod tests {
    use super::policy_for;
    use gent_ports::{PolicyLedger, WorkspaceLedger};
    use gent_store::SqliteLedger;
    use gent_types::{PermissionMode, PolicyRecord, PolicyScope, WorkspaceRecord};

    #[test]
    fn a_workspace_policy_revision_gates_its_next_request() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let initial = policy_for(&ledger, "workspace-1").unwrap();
        assert_eq!(
            (initial.revision, initial.mode),
            (1, PermissionMode::AskEveryTime)
        );
        ledger
            .save_policy(&PolicyRecord {
                policy_id: "workspace-revision-2".into(),
                workspace_id: "workspace-1".into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 2,
                mode: PermissionMode::Autonomous,
                allowed_tools: Vec::new(),
                allowed_categories: Vec::new(),
            })
            .unwrap();
        let current = policy_for(&ledger, "workspace-1").unwrap();
        assert_eq!(
            (current.revision, current.mode),
            (2, PermissionMode::Autonomous)
        );
    }
}
