use gent_types::{
    PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, SandboxEnforcement,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Prompt,
    Deny,
    SandboxRequired,
}

#[must_use]
pub fn evaluate_permission(
    policy: &PolicyRecord,
    request: &PermissionRequest,
) -> PermissionDecision {
    evaluate_permission_with_sandbox(policy, request, SandboxEnforcement::Unavailable)
}

#[must_use]
pub fn evaluate_permission_with_sandbox(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    sandbox: SandboxEnforcement,
) -> PermissionDecision {
    if policy.unattended_requires_sandbox() && sandbox != SandboxEnforcement::Enforced {
        return PermissionDecision::SandboxRequired;
    }
    if mode_allows(policy.mode, request.category)
        || policy.allowed_tools.contains(&request.tool_name)
        || policy.allowed_categories.contains(&request.category)
        || policy
            .allowed_commands
            .iter()
            .any(|grant| gent_types::command_grant_permits(grant, request))
    {
        PermissionDecision::Allow
    } else {
        PermissionDecision::Prompt
    }
}

fn mode_allows(mode: PermissionMode, category: PermissionCategory) -> bool {
    match mode {
        PermissionMode::AskEveryTime => {
            matches!(
                category,
                PermissionCategory::Read | PermissionCategory::Network
            )
        }
        PermissionMode::AutoAcceptEdits => matches!(
            category,
            PermissionCategory::Read | PermissionCategory::Edit | PermissionCategory::Network
        ),
        PermissionMode::Autonomous => matches!(
            category,
            PermissionCategory::Read
                | PermissionCategory::Edit
                | PermissionCategory::Command
                | PermissionCategory::Network
        ),
        PermissionMode::Bypass => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionDecision, evaluate_permission, evaluate_permission_with_sandbox};
    use gent_types::{
        PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, PolicyScope,
        SandboxEnforcement,
    };

    fn policy(mode: PermissionMode) -> PolicyRecord {
        PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-1".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec!["git:status".into()],
            allowed_categories: vec![PermissionCategory::Network],
            allowed_commands: Vec::new(),
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
    fn grants_do_not_depend_on_input_order() {
        let mut current = policy(PermissionMode::AskEveryTime);
        current.allowed_tools = vec!["Write".into(), "Bash".into(), "Read".into()];
        current.allowed_categories = vec![PermissionCategory::Network, PermissionCategory::Edit];
        assert_eq!(
            evaluate_permission(&current, &request("Write", PermissionCategory::Command)),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(&current, &request("other", PermissionCategory::Network)),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(&current, &request("other", PermissionCategory::Command)),
            PermissionDecision::Prompt
        );
        current.mode = PermissionMode::Bypass;
        assert_eq!(
            evaluate_permission(&current, &request("Write", PermissionCategory::Command)),
            PermissionDecision::SandboxRequired
        );
    }

    #[test]
    fn exact_and_category_approvals_prevent_repeated_prompts_in_ask_every_time_mode() {
        let policy = policy(PermissionMode::AskEveryTime);
        assert_eq!(
            evaluate_permission(&policy, &request("git:status", PermissionCategory::Command)),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(&policy, &request("http:get", PermissionCategory::Network)),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(&policy, &request("shell", PermissionCategory::Command)),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn increasingly_explicit_modes_expand_only_their_documented_categories() {
        let mut autonomous = policy(PermissionMode::Autonomous);
        autonomous.allowed_categories.clear();
        assert_eq!(
            evaluate_permission(
                &policy(PermissionMode::AutoAcceptEdits),
                &request("edit", PermissionCategory::Edit)
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(&autonomous, &request("shell", PermissionCategory::Command)),
            PermissionDecision::SandboxRequired
        );
        assert_eq!(
            evaluate_permission_with_sandbox(
                &autonomous,
                &request("shell", PermissionCategory::Command),
                SandboxEnforcement::Enforced,
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission_with_sandbox(
                &autonomous,
                &request("http", PermissionCategory::Network),
                SandboxEnforcement::Enforced,
            ),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn bypass_never_uses_a_stored_grant_to_escape_missing_containment() {
        let policy = policy(PermissionMode::Bypass);
        let request = request("http:post", PermissionCategory::Network);
        assert_eq!(
            evaluate_permission(&policy, &request),
            PermissionDecision::SandboxRequired
        );
        assert_eq!(
            evaluate_permission_with_sandbox(&policy, &request, SandboxEnforcement::Enforced),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn an_unsandboxed_conversation_applies_its_broad_mode_without_containment_evidence() {
        let policy =
            policy(PermissionMode::Bypass).for_conversation(gent_types::Containment::Unsandboxed);
        assert_eq!(
            evaluate_permission(&policy, &request("http:post", PermissionCategory::Network)),
            PermissionDecision::Allow
        );
        let ask = policy_with(
            PermissionMode::AskEveryTime,
            gent_types::Containment::Sandboxed,
        );
        assert_eq!(
            evaluate_permission(&ask, &request("shell", PermissionCategory::Command)),
            PermissionDecision::Prompt
        );
    }

    fn policy_with(mode: PermissionMode, containment: gent_types::Containment) -> PolicyRecord {
        policy(mode).for_conversation(containment)
    }

    #[test]
    fn ask_every_time_only_asks_for_commands_and_edits() {
        let mut ask_every_time = policy(PermissionMode::AskEveryTime);
        ask_every_time.allowed_tools.clear();
        ask_every_time.allowed_categories.clear();
        for (category, expected) in [
            (PermissionCategory::Read, PermissionDecision::Allow),
            (PermissionCategory::Network, PermissionDecision::Allow),
            (PermissionCategory::Edit, PermissionDecision::Prompt),
            (PermissionCategory::Command, PermissionDecision::Prompt),
        ] {
            assert_eq!(
                evaluate_permission(&ask_every_time, &request("tool", category)),
                expected,
                "{category:?}"
            );
        }
    }

    #[test]
    fn provider_tools_follow_the_mode_table_whatever_their_name() {
        for mode in [
            PermissionMode::AskEveryTime,
            PermissionMode::AutoAcceptEdits,
            PermissionMode::Autonomous,
            PermissionMode::Bypass,
        ] {
            let mut current = policy(mode);
            current.allowed_tools.clear();
            current.allowed_categories.clear();
            for sandbox in [
                SandboxEnforcement::Unavailable,
                SandboxEnforcement::Enforced,
            ] {
                let expected = match (mode, sandbox) {
                    (
                        PermissionMode::Autonomous | PermissionMode::Bypass,
                        SandboxEnforcement::Unavailable,
                    ) => PermissionDecision::SandboxRequired,
                    (PermissionMode::Bypass, _) => PermissionDecision::Allow,
                    _ => PermissionDecision::Prompt,
                };
                for tool in [
                    "Agent",
                    "ScheduleWakeup",
                    "UnknownTool",
                    "mcp__server__tool",
                ] {
                    assert_eq!(
                        evaluate_permission_with_sandbox(
                            &current,
                            &request(tool, PermissionCategory::Provider),
                            sandbox
                        ),
                        expected,
                        "{tool} {mode:?} {sandbox:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn auto_accept_edits_only_asks_for_commands() {
        let mut auto_accept_edits = policy(PermissionMode::AutoAcceptEdits);
        auto_accept_edits.allowed_tools.clear();
        auto_accept_edits.allowed_categories.clear();
        for (category, expected) in [
            (PermissionCategory::Read, PermissionDecision::Allow),
            (PermissionCategory::Edit, PermissionDecision::Allow),
            (PermissionCategory::Network, PermissionDecision::Allow),
            (PermissionCategory::Command, PermissionDecision::Prompt),
        ] {
            assert_eq!(
                evaluate_permission(&auto_accept_edits, &request("tool", category)),
                expected,
                "{category:?}"
            );
        }
    }
}
