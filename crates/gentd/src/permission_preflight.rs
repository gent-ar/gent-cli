use gent_core::{PermissionDecision, evaluate_permission_with_sandbox};
use gent_types::{
    AgentChatMode, PermissionCategory, PermissionDenialReason, PermissionRequest, PolicyRecord,
    SandboxEnforcement,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModeGate {
    Policy,
    AskUser,
    Refuse(PermissionDenialReason),
}

pub(crate) const fn mode_gate(mode: AgentChatMode, request: &PermissionRequest) -> ModeGate {
    match (mode, request.category) {
        (AgentChatMode::Plan, PermissionCategory::Edit) => {
            ModeGate::Refuse(PermissionDenialReason::PlanModeEdit)
        }
        (AgentChatMode::Ask, PermissionCategory::Edit | PermissionCategory::Command) => {
            ModeGate::AskUser
        }
        _ => ModeGate::Policy,
    }
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
    let enforcement = evidence
        .filter(|proof| proof.run_id() == run_id)
        .and_then(|proof| proof.permission_binding())
        .filter(|binding| binding.run_id() == run_id && binding.policy() == policy)
        .map_or(SandboxEnforcement::Unavailable, |_| {
            SandboxEnforcement::Enforced
        });
    evaluate_policy(policy, request, enforcement, false)
}

pub(crate) fn evaluate_policy(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    enforcement: SandboxEnforcement,
    trusted_internal: bool,
) -> PermissionPreflight {
    if crate::permission_category::requires_user_response(&request.tool_name) {
        return PermissionPreflight::Ask;
    }
    if crate::permission_category::stays_in_session(&request.tool_name) {
        return PermissionPreflight::Allow;
    }
    if trusted_internal
        && (!policy.unattended_requires_sandbox() || enforcement == SandboxEnforcement::Enforced)
    {
        return PermissionPreflight::Allow;
    }
    match evaluate_permission_with_sandbox(policy, request, enforcement) {
        PermissionDecision::Allow => PermissionPreflight::Allow,
        PermissionDecision::Deny => PermissionPreflight::Deny,
        PermissionDecision::Prompt | PermissionDecision::SandboxRequired => {
            PermissionPreflight::Ask
        }
    }
}

pub(crate) fn evaluate_callback(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    run: &str,
    conversation: &str,
    epoch: gent_types::HostEpoch,
    evidence: Option<&gent_drivers::ProcessSandboxEvidence>,
    trusted_internal: bool,
) -> PermissionPreflight {
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
    let enforcement = if evidence.is_some() {
        SandboxEnforcement::Enforced
    } else {
        SandboxEnforcement::Unavailable
    };
    evaluate_policy(policy, request, enforcement, trusted_internal)
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
    fn session_internal_tools_never_ask_and_never_need_a_sandbox() {
        use gent_types::{Containment, SandboxEnforcement};
        for mode in [
            PermissionMode::AskEveryTime,
            PermissionMode::AutoAcceptEdits,
            PermissionMode::Autonomous,
            PermissionMode::Bypass,
        ] {
            for containment in [Containment::Sandboxed, Containment::Unsandboxed] {
                let current = policy(mode).for_conversation(containment);
                for enforcement in [
                    SandboxEnforcement::Unavailable,
                    SandboxEnforcement::Enforced,
                ] {
                    let outcome = |tool: &str| {
                        super::evaluate_policy(
                            &current,
                            &request(tool, PermissionCategory::Provider),
                            enforcement,
                            false,
                        )
                    };
                    for tool in ["Agent", "Task", "ScheduleWakeup", "TodoWrite"] {
                        assert_eq!(outcome(tool), PermissionPreflight::Allow, "{tool} {mode:?}");
                    }
                    let ordinary = if mode == PermissionMode::Bypass
                        && (containment == Containment::Unsandboxed
                            || enforcement == SandboxEnforcement::Enforced)
                    {
                        PermissionPreflight::Allow
                    } else {
                        PermissionPreflight::Ask
                    };
                    for tool in ["UnknownTool", "mcp__server__Agent"] {
                        assert_eq!(outcome(tool), ordinary, "{tool} {mode:?} {containment:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn plan_mode_refuses_only_edits_and_leaves_commands_to_the_policy() {
        use super::{ModeGate, mode_gate};
        use gent_types::{AgentChatMode, PermissionDenialReason};
        for (mode, category, gate) in [
            (
                AgentChatMode::Plan,
                PermissionCategory::Edit,
                ModeGate::Refuse(PermissionDenialReason::PlanModeEdit),
            ),
            (
                AgentChatMode::Plan,
                PermissionCategory::Command,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Plan,
                PermissionCategory::Read,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Plan,
                PermissionCategory::Network,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Plan,
                PermissionCategory::Provider,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Ask,
                PermissionCategory::Edit,
                ModeGate::AskUser,
            ),
            (
                AgentChatMode::Ask,
                PermissionCategory::Command,
                ModeGate::AskUser,
            ),
            (
                AgentChatMode::Ask,
                PermissionCategory::Read,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Agent,
                PermissionCategory::Edit,
                ModeGate::Policy,
            ),
            (
                AgentChatMode::Agent,
                PermissionCategory::Command,
                ModeGate::Policy,
            ),
        ] {
            assert_eq!(
                mode_gate(mode, &request("tool", category)),
                gate,
                "{mode:?} {category:?}"
            );
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
