use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyScope {
    ProviderPermissions,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    AskEveryTime,
    AutoAcceptEdits,
    Autonomous,
    Bypass,
}

impl PermissionMode {
    #[must_use]
    pub const fn is_unattended(self) -> bool {
        matches!(self, Self::Autonomous | Self::Bypass)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Containment {
    #[default]
    Unsandboxed,
    Sandboxed,
}

impl Containment {
    #[must_use]
    pub const fn requires_sandbox(self) -> bool {
        matches!(self, Self::Sandboxed)
    }

    #[must_use]
    pub const fn unresolved() -> Self {
        Self::Sandboxed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEnforcement {
    Enforced,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionCategory {
    Read,
    Edit,
    Command,
    Network,
    Provider,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub tool_name: String,
    pub category: PermissionCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_id: Option<String>,
}

impl PermissionRequest {
    #[must_use]
    pub const fn new(
        tool_name: String,
        category: PermissionCategory,
        input: Option<serde_json::Value>,
        child_id: Option<String>,
    ) -> Self {
        Self {
            tool_name,
            category,
            input,
            child_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionDenialReason {
    PlanModeEdit,
}

impl PermissionDenialReason {
    #[must_use]
    pub const fn notice(self) -> &'static str {
        match self {
            Self::PlanModeEdit => {
                "Gent denied a file change because Plan mode does not edit project files. Start implementing the plan to allow changes."
            }
        }
    }

    #[must_use]
    pub const fn provider_message(self) -> &'static str {
        match self {
            Self::PlanModeEdit => {
                "This conversation is in Plan mode: project files are not edited until the user starts implementing the plan. Commands, reads, web research and sub-agents remain available; put this change in the plan instead."
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRecord {
    pub policy_id: String,
    pub workspace_id: String,
    pub scope: PolicyScope,
    pub revision: u64,
    #[serde(default)]
    pub mode: PermissionMode,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub allowed_categories: Vec<PermissionCategory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_commands: Vec<crate::CommandGrant>,
    #[serde(skip, default = "Containment::unresolved")]
    pub containment: Containment,
}

impl PolicyRecord {
    #[must_use]
    pub const fn requires_sandbox(&self) -> bool {
        self.containment.requires_sandbox()
    }

    #[must_use]
    pub const fn unattended_requires_sandbox(&self) -> bool {
        self.mode.is_unattended() && self.requires_sandbox()
    }

    #[must_use]
    pub fn for_conversation(mut self, containment: Containment) -> Self {
        self.containment = containment;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Containment, PermissionCategory, PermissionMode, PolicyRecord, PolicyScope,
        SandboxEnforcement,
    };
    use serde_json::json;

    #[test]
    fn older_policy_json_defaults_to_the_least_permissive_mode() {
        let policy: PolicyRecord = serde_json::from_value(json!({
            "policyId": "policy-1", "workspaceId": "workspace-1",
            "scope": "providerPermissions", "revision": 1,
            "allowedTools": ["git:status"]
        }))
        .unwrap();
        assert_eq!(policy.mode, PermissionMode::AskEveryTime);
        assert!(policy.allowed_categories.is_empty());
    }

    #[test]
    fn category_names_are_stable_and_provider_neutral() {
        assert_eq!(
            serde_json::to_value(PermissionCategory::Network).unwrap(),
            json!("network")
        );
        assert_eq!(
            serde_json::to_value(PermissionMode::AutoAcceptEdits).unwrap(),
            json!("autoAcceptEdits")
        );
        let _ = PolicyScope::ProviderPermissions;
    }

    #[test]
    fn only_a_sandboxed_conversation_requires_a_daemon_verified_sandbox() {
        assert!(PermissionMode::Autonomous.is_unattended());
        assert!(PermissionMode::Bypass.is_unattended());
        assert!(!PermissionMode::AskEveryTime.is_unattended());
        assert!(Containment::Sandboxed.requires_sandbox());
        assert!(!Containment::Unsandboxed.requires_sandbox());
        assert_eq!(Containment::default(), Containment::Unsandboxed);
        assert_eq!(
            serde_json::to_value(Containment::Sandboxed).unwrap(),
            json!("sandboxed")
        );
        assert_eq!(
            serde_json::to_value(Containment::Unsandboxed).unwrap(),
            json!("unsandboxed")
        );
        let _ = SandboxEnforcement::Enforced;
    }

    #[test]
    fn workspace_policy_json_never_carries_conversation_containment() {
        let policy: PolicyRecord = serde_json::from_value(json!({
            "policyId": "policy-1", "workspaceId": "workspace-1",
            "scope": "providerPermissions", "revision": 1, "mode": "bypass",
            "containment": "unsandboxed"
        }))
        .unwrap();
        assert_eq!(policy.containment, Containment::Sandboxed);
        assert!(
            serde_json::to_value(&policy)
                .unwrap()
                .get("containment")
                .is_none()
        );
        assert!(
            !policy
                .for_conversation(Containment::Unsandboxed)
                .requires_sandbox()
        );
    }
}
