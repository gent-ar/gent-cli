//! Pure evaluation of durable user permission policy.

use gent_types::{
    PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, SandboxEnforcement,
};

/// The only outcomes a future provider ingress may act upon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Prompt,
    Deny,
    SandboxRequired,
}

/// Evaluates one typed request without I/O, provider knowledge, or mutable state.
#[must_use]
pub fn evaluate_permission(
    policy: &PolicyRecord,
    request: &PermissionRequest,
) -> PermissionDecision {
    evaluate_permission_with_sandbox(policy, request, SandboxEnforcement::Unavailable)
}

/// Evaluates one request with a daemon-verified OS-sandbox result.
///
/// A broad unattended policy fails closed before any exact or category approval when containment
/// is unavailable. This prevents a future provider launch from treating a client setting as proof
/// that it may run unsandboxed.
#[must_use]
pub fn evaluate_permission_with_sandbox(
    policy: &PolicyRecord,
    request: &PermissionRequest,
    sandbox: SandboxEnforcement,
) -> PermissionDecision {
    if policy.mode == PermissionMode::Plan && request.category != PermissionCategory::Read {
        return PermissionDecision::Deny;
    }
    if policy.mode.requires_sandbox() && sandbox != SandboxEnforcement::Enforced {
        return PermissionDecision::SandboxRequired;
    }
    if mode_allows(policy.mode, request.category)
        || policy
            .allowed_tools
            .binary_search(&request.tool_name)
            .is_ok()
        || policy
            .allowed_categories
            .binary_search(&request.category)
            .is_ok()
    {
        PermissionDecision::Allow
    } else {
        PermissionDecision::Prompt
    }
}

fn mode_allows(mode: PermissionMode, category: PermissionCategory) -> bool {
    match mode {
        PermissionMode::Default => false,
        PermissionMode::Plan => category == PermissionCategory::Read,
        PermissionMode::AutoAcceptEdits => matches!(
            category,
            PermissionCategory::Read | PermissionCategory::Edit
        ),
        PermissionMode::Autonomous => matches!(
            category,
            PermissionCategory::Read | PermissionCategory::Edit | PermissionCategory::Command
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
            allowed_tools: vec!["git:status".into()],
            allowed_categories: vec![PermissionCategory::Network],
        }
    }

    fn request(tool_name: &str, category: PermissionCategory) -> PermissionRequest {
        PermissionRequest {
            tool_name: tool_name.into(),
            category,
        }
    }

    #[test]
    fn plan_mode_stays_plan_even_when_the_policy_has_broader_approvals() {
        let policy = policy(PermissionMode::Plan);
        assert_eq!(
            evaluate_permission(&policy, &request("git:status", PermissionCategory::Command)),
            PermissionDecision::Deny
        );
        assert_eq!(
            evaluate_permission(
                &policy,
                &request("workspace:search", PermissionCategory::Read)
            ),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn exact_and_category_approvals_prevent_repeated_prompts_in_default_mode() {
        let policy = policy(PermissionMode::Default);
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
            PermissionDecision::Prompt
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
}
