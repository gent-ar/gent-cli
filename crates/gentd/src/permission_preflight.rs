use gent_core::{PermissionDecision, evaluate_permission_with_sandbox};
use gent_types::{PermissionRequest, PolicyRecord, SandboxEnforcement};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionPreflight {
    Allow,
    Deny,
    Ask,
}

pub(crate) fn evaluate(policy: &PolicyRecord, request: &PermissionRequest) -> PermissionPreflight {
    match evaluate_permission_with_sandbox(policy, request, SandboxEnforcement::Unavailable) {
        PermissionDecision::Allow => PermissionPreflight::Allow,
        PermissionDecision::Deny => PermissionPreflight::Deny,
        PermissionDecision::Prompt | PermissionDecision::SandboxRequired => {
            PermissionPreflight::Ask
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{
        PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, PolicyScope,
    };

    use super::{PermissionPreflight, evaluate};

    fn policy(mode: PermissionMode) -> PolicyRecord {
        PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode,
            allowed_tools: vec!["workspace:edit".into()],
            allowed_categories: vec![PermissionCategory::Network],
        }
    }

    fn request(tool_name: &str, category: PermissionCategory) -> PermissionRequest {
        PermissionRequest {
            tool_name: tool_name.into(),
            category,
            input: None,
        }
    }

    #[test]
    fn exact_and_category_grants_are_auto_approved_in_ask_mode() {
        assert_eq!(
            evaluate(
                &policy(PermissionMode::Default),
                &request("workspace:edit", PermissionCategory::Edit)
            ),
            PermissionPreflight::Allow
        );
        assert_eq!(
            evaluate(
                &policy(PermissionMode::Default),
                &request("fetch", PermissionCategory::Network)
            ),
            PermissionPreflight::Allow
        );
    }

    #[test]
    fn plan_mode_denies_changes_even_when_a_workspace_grant_exists() {
        assert_eq!(
            evaluate(
                &policy(PermissionMode::Plan),
                &request("workspace:edit", PermissionCategory::Edit)
            ),
            PermissionPreflight::Deny
        );
    }

    #[test]
    fn edits_mode_only_auto_approves_edits() {
        assert_eq!(
            evaluate(
                &policy(PermissionMode::AutoAcceptEdits),
                &request("workspace:edit", PermissionCategory::Edit)
            ),
            PermissionPreflight::Allow
        );
        assert_eq!(
            evaluate(
                &policy(PermissionMode::AutoAcceptEdits),
                &request("shell", PermissionCategory::Command)
            ),
            PermissionPreflight::Ask
        );
    }

    #[test]
    fn autonomous_and_bypass_require_a_user_decision_without_sandbox_enforcement() {
        for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
            for category in [
                PermissionCategory::Read,
                PermissionCategory::Edit,
                PermissionCategory::Command,
                PermissionCategory::Network,
                PermissionCategory::Provider,
            ] {
                assert_eq!(
                    evaluate(&policy(mode), &request("tool", category)),
                    PermissionPreflight::Ask
                );
            }
        }
    }
}
