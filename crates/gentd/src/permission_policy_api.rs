//! Daemon mapping for secret-free permission-policy revisions.

use gent_ports::{Ledger, PolicyLedger, WorkspaceLedger};
use gent_protocol::PermissionPolicyFrame;
use gent_runtime::Coordinator;
use gent_types::{PermissionMode, PolicyScope};

pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    frame: PermissionPolicyFrame,
) -> Result<PermissionPolicyFrame, String>
where
    L: Ledger + PolicyLedger + WorkspaceLedger,
{
    match frame {
        PermissionPolicyFrame::Current {
            request_id,
            workspace_id,
        } => current(coordinator, &workspace_id)
            .map(|policy| PermissionPolicyFrame::CurrentPolicy { request_id, policy })
            .map_err(|error| error.to_string()),
        PermissionPolicyFrame::Save {
            request_id,
            policy,
            bypass_consent,
            ..
        } => save(coordinator, request_id, policy, bypass_consent),
        PermissionPolicyFrame::CurrentPolicy { .. } | PermissionPolicyFrame::Saved { .. } => {
            Err("permission policy response frames are server-only".into())
        }
    }
}

fn current<L>(
    coordinator: &Coordinator<L>,
    workspace_id: &str,
) -> Result<Option<gent_types::PolicyRecord>, gent_runtime::RuntimeError>
where
    L: Ledger + PolicyLedger + WorkspaceLedger,
{
    if coordinator.workspace(workspace_id)?.is_none() {
        return coordinator.current_policy(workspace_id, PolicyScope::ProviderPermissions);
    }
    coordinator
        .ensure_default_provider_permission_policy(workspace_id)
        .map(Some)
}

fn save<L>(
    coordinator: &Coordinator<L>,
    request_id: String,
    policy: gent_types::PolicyRecord,
    bypass_consent: bool,
) -> Result<PermissionPolicyFrame, String>
where
    L: Ledger + PolicyLedger,
{
    if policy.mode == PermissionMode::Bypass && !bypass_consent {
        return Err("changing to bypass mode requires explicit bypass consent".into());
    }
    if coordinator
        .current_policy(&policy.workspace_id, policy.scope)
        .map_err(|error| error.to_string())?
        .as_ref()
        == Some(&policy)
    {
        return Ok(PermissionPolicyFrame::Saved { request_id, policy });
    }
    coordinator
        .save_provider_permission_policy(&policy)
        .map_err(|error| error.to_string())?;
    Ok(PermissionPolicyFrame::Saved { request_id, policy })
}

#[cfg(test)]
mod tests {
    use super::exchange;
    use gent_protocol::PermissionPolicyFrame;
    use gent_runtime::Coordinator;
    use gent_store::SqliteLedger;
    use gent_types::{CapabilitySet, PermissionMode, PolicyRecord, PolicyScope, WorkspaceRecord};

    fn policy(mode: PermissionMode) -> PolicyRecord {
        PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-1".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: Vec::new(),
            allowed_categories: Vec::new(),
        }
    }

    #[test]
    fn broad_modes_save_without_probing_a_sandbox_because_containment_is_per_conversation() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        coordinator
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        coordinator
            .save_policy(&policy(PermissionMode::AutoAcceptEdits))
            .unwrap();
        let mut next = policy(PermissionMode::Bypass);
        next.policy_id = "policy-2".into();
        next.revision = 2;
        let unconsented = exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request".into(),
                policy: next.clone(),
                bypass_consent: false,
                expected_provider: None,
            },
        );
        assert!(unconsented.unwrap_err().contains("bypass consent"));
        exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request".into(),
                policy: next,
                bypass_consent: true,
                expected_provider: None,
            },
        )
        .unwrap();
        let current = coordinator
            .current_policy("workspace-1", PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap();
        assert_eq!(
            (current.mode, current.revision),
            (PermissionMode::Bypass, 2)
        );
    }

    #[test]
    fn bypass_still_requires_explicit_consent() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        let error = exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request".into(),
                policy: policy(PermissionMode::Bypass),
                bypass_consent: false,
                expected_provider: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("requires explicit"));
    }

    #[test]
    fn supported_mode_is_saved_and_returned() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        coordinator
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let reply = exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request".into(),
                policy: policy(PermissionMode::AutoAcceptEdits),
                bypass_consent: false,
                expected_provider: None,
            },
        )
        .unwrap();
        assert!(
            matches!(reply, PermissionPolicyFrame::Saved {policy, ..} if policy.mode == PermissionMode::AutoAcceptEdits)
        );
    }
    #[test]
    fn conservative_default_is_idempotent_and_an_explicit_revision_remains_authoritative() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        coordinator
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let first = coordinator
            .ensure_default_provider_permission_policy("workspace-1")
            .unwrap();
        let retry = coordinator
            .ensure_default_provider_permission_policy("workspace-1")
            .unwrap();
        assert_eq!(first, retry);
        assert_eq!(first.mode, PermissionMode::AskEveryTime);
        assert!(first.allowed_tools.is_empty() && first.allowed_categories.is_empty());
        let revised = PolicyRecord {
            revision: 2,
            policy_id: "policy-2".into(),
            mode: PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Sandboxed,
            ..first
        };
        coordinator.save_policy(&revised).unwrap();
        assert_eq!(
            coordinator
                .ensure_default_provider_permission_policy("workspace-1")
                .unwrap(),
            revised
        );
    }

    #[test]
    fn a_known_workspace_reports_the_policy_that_gates_its_chats() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        coordinator
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let current = |workspace_id: &str| match exchange(
            &coordinator,
            PermissionPolicyFrame::Current {
                request_id: "request".into(),
                workspace_id: workspace_id.into(),
            },
        )
        .unwrap()
        {
            PermissionPolicyFrame::CurrentPolicy { policy, .. } => policy,
            _ => panic!("current policy reply"),
        };
        let policy = current("workspace-1").unwrap();
        assert_eq!(
            (policy.revision, policy.mode),
            (1, PermissionMode::AskEveryTime)
        );
        assert_eq!(
            crate::permission_workspace::policy_for(&ledger, "workspace-1").unwrap(),
            policy
        );
        assert_eq!(current("workspace-unknown"), None);
    }
}
