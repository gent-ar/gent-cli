//! Daemon mapping for secret-free permission-policy revisions.

use gent_ports::{Ledger, PolicyLedger};
use gent_protocol::PermissionPolicyFrame;
use gent_runtime::Coordinator;
use gent_types::{PermissionMode, PolicyScope};

/// Handles one local policy exchange without provider, process, or terminal dependencies.
pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    frame: PermissionPolicyFrame,
) -> Result<PermissionPolicyFrame, String>
where
    L: Ledger + PolicyLedger,
{
    match frame {
        PermissionPolicyFrame::Current {
            request_id,
            workspace_id,
        } => coordinator
            .current_policy(&workspace_id, PolicyScope::ProviderPermissions)
            .map(|policy| PermissionPolicyFrame::CurrentPolicy { request_id, policy })
            .map_err(|error| error.to_string()),
        PermissionPolicyFrame::Save {
            request_id,
            policy,
            bypass_consent,
        } => save(coordinator, request_id, policy, bypass_consent),
        PermissionPolicyFrame::CurrentPolicy { .. } | PermissionPolicyFrame::Saved { .. } => {
            Err("permission policy response frames are server-only".into())
        }
    }
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
    coordinator
        .save_policy(&policy)
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
            allowed_tools: Vec::new(),
            allowed_categories: Vec::new(),
        }
    }

    #[test]
    fn bypass_requires_one_time_consent_then_persists_as_the_selected_mode() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        coordinator
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let denied = exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request-1".into(),
                policy: policy(PermissionMode::Bypass),
                bypass_consent: false,
            },
        );
        assert!(denied.unwrap_err().contains("requires explicit"));
        let saved = exchange(
            &coordinator,
            PermissionPolicyFrame::Save {
                request_id: "request-2".into(),
                policy: policy(PermissionMode::Bypass),
                bypass_consent: true,
            },
        )
        .unwrap();
        assert!(
            matches!(saved, PermissionPolicyFrame::Saved { policy, .. } if policy.mode == PermissionMode::Bypass)
        );
        let current = exchange(
            &coordinator,
            PermissionPolicyFrame::Current {
                request_id: "request-3".into(),
                workspace_id: "workspace-1".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            current,
            PermissionPolicyFrame::CurrentPolicy {
                policy: Some(policy),
                ..
            } if policy.mode == PermissionMode::Bypass
        ));
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
        assert_eq!(first.mode, PermissionMode::Default);
        assert!(first.allowed_tools.is_empty() && first.allowed_categories.is_empty());
        let revised = PolicyRecord {
            revision: 2,
            policy_id: "policy-2".into(),
            mode: PermissionMode::Plan,
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
}
