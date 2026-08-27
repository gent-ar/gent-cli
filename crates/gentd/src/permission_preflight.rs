use gent_types::{PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionPreflight {
    Allow,
    Deny,
    Ask,
}

pub(crate) fn evaluate(policy: &PolicyRecord, request: &PermissionRequest) -> PermissionPreflight {
    if policy.mode == PermissionMode::Plan {
        return if request.category == PermissionCategory::Read {
            PermissionPreflight::Allow
        } else {
            PermissionPreflight::Deny
        };
    }
    if matches!(
        policy.mode,
        PermissionMode::Autonomous | PermissionMode::Bypass
    ) {
        return PermissionPreflight::Allow;
    }
    if policy.mode == PermissionMode::AutoAcceptEdits
        && request.category == PermissionCategory::Edit
    {
        return PermissionPreflight::Allow;
    }
    if policy
        .allowed_tools
        .binary_search(&request.tool_name)
        .is_ok()
        || policy
            .allowed_categories
            .binary_search(&request.category)
            .is_ok()
    {
        PermissionPreflight::Allow
    } else {
        PermissionPreflight::Ask
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
    fn autonomous_and_bypass_allow_each_category() {
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
                    PermissionPreflight::Allow
                );
            }
        }
    }
}
