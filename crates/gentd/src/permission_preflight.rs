use gent_core::{PermissionDecision, evaluate_permission_with_sandbox};
use gent_types::{PermissionRequest, PolicyRecord, SandboxEnforcement};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionPreflight {
    Allow,
    Deny,
    Ask,
}

pub(crate) fn evaluate(policy: &PolicyRecord, request: &PermissionRequest) -> PermissionPreflight {
    if is_internal_tool(&request.tool_name) {
        return PermissionPreflight::Allow;
    }
    match evaluate_permission_with_sandbox(policy, request, SandboxEnforcement::Unavailable) {
        PermissionDecision::Allow => PermissionPreflight::Allow,
        PermissionDecision::Deny => PermissionPreflight::Deny,
        PermissionDecision::Prompt | PermissionDecision::SandboxRequired => {
            PermissionPreflight::Ask
        }
    }
}

pub(crate) fn is_internal_tool(tool_name: &str) -> bool {
    tool_name.strip_prefix("mcp__").is_some_and(|qualified| {
        let server = qualified
            .split_once("__")
            .map_or(qualified, |(server, _)| server);
        crate::standalone_mcp_config::INTERNAL_SERVER_NAMES
            .iter()
            .any(|name| server == *name || server == name.replace('-', "_"))
    })
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
            child_id: None,
        }
    }

    #[test]
    fn exact_and_category_grants_are_auto_approved_in_ask_mode() {
        assert_eq!(
            evaluate(
                &policy(PermissionMode::AskEveryTime),
                &request("workspace:edit", PermissionCategory::Edit)
            ),
            PermissionPreflight::Allow
        );
        assert_eq!(
            evaluate(
                &policy(PermissionMode::AskEveryTime),
                &request("fetch", PermissionCategory::Network)
            ),
            PermissionPreflight::Allow
        );
    }

    #[test]
    fn gent_internal_mcp_tools_are_pre_approved_in_every_mode() {
        for mode in [PermissionMode::AskEveryTime, PermissionMode::Bypass] {
            for tool in [
                "mcp__gent-goal__gent_goal_update",
                "mcp__gent_goal__gent_goal_update",
                "mcp__gent-automations__list",
                "mcp__gent-forge",
            ] {
                assert_eq!(
                    evaluate(&policy(mode), &request(tool, PermissionCategory::Provider)),
                    PermissionPreflight::Allow
                );
            }
        }
        for lookalike in [
            "mcp__gent-goal-extra__gent_goal_update",
            "mcp__other__gent_goal_update",
            "gent-goal",
        ] {
            assert_eq!(
                evaluate(
                    &policy(PermissionMode::AskEveryTime),
                    &request(lookalike, PermissionCategory::Provider)
                ),
                PermissionPreflight::Ask
            );
        }
    }

    #[test]
    fn ask_mode_requires_a_decision_without_a_workspace_grant() {
        assert_eq!(
            evaluate(
                &policy(PermissionMode::AskEveryTime),
                &request("shell", PermissionCategory::Command)
            ),
            PermissionPreflight::Ask
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
