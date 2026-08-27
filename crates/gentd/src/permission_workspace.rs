//! Daemon-local durable namespace for user permission preferences.

use gent_ports::{PolicyLedger, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{PolicyRecord, PolicyScope, WorkspaceRecord};

/// The local runtime settings are intentionally independent from Git workspaces.
pub(crate) const SETTINGS_WORKSPACE_ID: &str = "gent-local-settings";

/// Creates the one local settings identity exactly once; no provider or filesystem effect occurs.
pub(crate) fn ensure(ledger: &SqliteLedger, data_dir: &std::path::Path) -> Result<(), String> {
    if ledger
        .find_workspace(SETTINGS_WORKSPACE_ID)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: SETTINGS_WORKSPACE_ID.into(),
                canonical_path: data_dir
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .into_owned(),
            })
            .map_err(|error| error.to_string())?;
    }
    ledger
        .ensure_default_provider_permission_policy(SETTINGS_WORKSPACE_ID)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn policy_for<L>(
    ledger: &L,
    workspace_id: &str,
) -> Result<PolicyRecord, gent_ports::LedgerError>
where
    L: PolicyLedger,
{
    if let Some(policy) = ledger.current_policy(workspace_id, PolicyScope::ProviderPermissions)? {
        return Ok(policy);
    }
    let Some(global) =
        ledger.current_policy(SETTINGS_WORKSPACE_ID, PolicyScope::ProviderPermissions)?
    else {
        return ledger.ensure_default_provider_permission_policy(workspace_id);
    };
    let policy = PolicyRecord {
        policy_id: format!("permission-policy-{}", uuid::Uuid::new_v4()),
        workspace_id: workspace_id.into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: global.mode,
        allowed_tools: global.allowed_tools,
        allowed_categories: global.allowed_categories,
    };
    ledger.save_policy(&policy)?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::{SETTINGS_WORKSPACE_ID, policy_for};
    use gent_ports::{PolicyLedger, WorkspaceLedger};
    use gent_store::SqliteLedger;
    use gent_types::{
        PermissionCategory, PermissionMode, PolicyRecord, PolicyScope, WorkspaceRecord,
    };

    #[test]
    fn a_workspace_starts_with_the_global_permission_posture() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: SETTINGS_WORKSPACE_ID.into(),
                canonical_path: "/settings".into(),
            })
            .unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let global = ledger
            .ensure_default_provider_permission_policy(SETTINGS_WORKSPACE_ID)
            .unwrap();
        ledger
            .save_policy(&PolicyRecord {
                policy_id: "global-revision-2".into(),
                workspace_id: SETTINGS_WORKSPACE_ID.into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 2,
                mode: PermissionMode::AutoAcceptEdits,
                allowed_tools: vec!["git:status".into()],
                allowed_categories: vec![PermissionCategory::Read],
            })
            .unwrap();
        assert_eq!(global.revision, 1);
        let workspace = policy_for(&ledger, "workspace-1").unwrap();
        assert_eq!(workspace.mode, PermissionMode::AutoAcceptEdits);
        assert_eq!(workspace.allowed_tools, ["git:status"]);
        assert_eq!(workspace.allowed_categories, [PermissionCategory::Read]);
    }
}
