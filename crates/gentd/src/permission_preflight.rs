use gent_core::{PermissionDecision, evaluate_permission_with_sandbox};
use gent_types::{
    AgentChatMode, PermissionCategory, PermissionDenialReason, PermissionRequest, PolicyRecord,
    SandboxEnforcement, SandboxWorkspaceAccess,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionPreflight {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Copy)]
pub(crate) struct PreflightContext<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) conversation_id: &'a str,
    pub(crate) disallowed: &'a [String],
    pub(crate) injection: Option<&'a crate::standalone_mcp_config::InjectedInternalMcp>,
    pub(crate) host_epoch: gent_types::HostEpoch,
}

pub(crate) fn read_only_denial(
    mode: AgentChatMode,
    request: &PermissionRequest,
) -> Option<PermissionDenialReason> {
    (SandboxWorkspaceAccess::from_mode(mode) == SandboxWorkspaceAccess::ReadOnly
        && matches!(
            request.category,
            PermissionCategory::Edit | PermissionCategory::Command
        ))
    .then_some(PermissionDenialReason::ReadOnlyMode)
}

pub(crate) fn evaluate(policy: &PolicyRecord, request: &PermissionRequest) -> PermissionPreflight {
    evaluate_with_evidence(policy, request, "", None)
}

pub(crate) fn evaluate_with_evidence(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    run_id: &str,
    evidence: Option<&gent_drivers::ProcessSandboxEvidence>,
) -> PermissionPreflight {
    if crate::permission_category::requires_user_response(&request.tool_name) {
        return PermissionPreflight::Ask;
    }
    let enforcement = evidence
        .filter(|proof| proof.run_id() == run_id)
        .and_then(|proof| proof.permission_binding())
        .filter(|binding| binding.run_id() == run_id && binding.policy() == policy)
        .map_or(SandboxEnforcement::Unavailable, |_| {
            SandboxEnforcement::Enforced
        });
    match evaluate_permission_with_sandbox(policy, request, enforcement) {
        PermissionDecision::Allow => PermissionPreflight::Allow,
        PermissionDecision::Deny => PermissionPreflight::Deny,
        PermissionDecision::Prompt | PermissionDecision::SandboxRequired => {
            PermissionPreflight::Ask
        }
    }
}

/// Process evidence is borrowed only while the production runner owner remains locked.
/// Injection provenance identifies a Gent control; it is never containment evidence.
pub(crate) fn evaluate_callback(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    run: &str,
    conversation: &str,
    epoch: gent_types::HostEpoch,
    evidence: Option<&gent_drivers::ProcessSandboxEvidence>,
    trusted_internal: bool,
) -> PermissionPreflight {
    if crate::permission_category::requires_user_response(&request.tool_name) {
        return PermissionPreflight::Ask;
    }
    let evidence = evidence
        .filter(|proof| proof.run_id() == run)
        .filter(|proof| {
            proof.permission_binding().is_some_and(|binding| {
                binding.run_id() == run
                    && binding.conversation_id() == conversation
                    && binding.policy() == policy
                    && binding.host_epoch() == epoch
            })
        });
    if trusted_internal && (!policy.unattended_requires_sandbox() || evidence.is_some()) {
        return PermissionPreflight::Allow;
    }
    evaluate_with_evidence(policy, request, run, evidence)
}

#[cfg(test)]
#[path = "permission_preflight_sandbox_tests.rs"]
mod sandbox_tests;

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
            containment: gent_types::Containment::Sandboxed,
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
    fn user_questions_are_never_auto_approved_even_with_explicit_grants() {
        for mode in [
            PermissionMode::AskEveryTime,
            PermissionMode::AutoAcceptEdits,
            PermissionMode::Autonomous,
            PermissionMode::Bypass,
        ] {
            let mut current = policy(mode);
            current.allowed_tools = vec!["AskUserQuestion".into()];
            current.allowed_categories = vec![PermissionCategory::Provider];
            for name in ["AskUserQuestion", "ask_user"] {
                let question = request(name, PermissionCategory::Provider);
                assert_eq!(
                    super::evaluate_with_evidence(&current, &question, "run", None),
                    PermissionPreflight::Ask
                );
                assert_eq!(
                    super::evaluate_callback(
                        &current,
                        &question,
                        "run",
                        "conversation",
                        gent_types::HostEpoch(1),
                        None,
                        true
                    ),
                    PermissionPreflight::Ask
                );
            }
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
    fn names_without_owned_injection_provenance_never_get_an_internal_exemption() {
        for mode in [PermissionMode::AskEveryTime, PermissionMode::Bypass] {
            for tool in [
                "mcp__gent-goal__gent_goal_update",
                "mcp__gent_goal__gent_goal_update",
                "mcp__gent-chat__unknown",
            ] {
                assert_eq!(
                    evaluate(&policy(mode), &request(tool, PermissionCategory::Provider)),
                    PermissionPreflight::Ask
                );
            }
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
