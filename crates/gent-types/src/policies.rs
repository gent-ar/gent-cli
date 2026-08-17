//! Versioned, secret-free workspace permission policy records.

use serde::{Deserialize, Serialize};

/// The policy domain covered by a record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyScope {
    ProviderPermissions,
}

/// The default behavior applied to a provider-neutral permission request.
///
/// A mode never starts a provider or bypasses the daemon's observer boundary. It is only a
/// durable user preference consumed by future authority-gated provider ingress.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Ask for every operation that has not received an explicit durable approval.
    #[default]
    Default,
    /// Permit read-only planning, while every non-read operation remains blocked by the mode.
    Plan,
    /// Permit reads and edits; commands, network access, and provider control still ask.
    AutoAcceptEdits,
    /// Permit reads, edits, and commands; network and provider control still ask.
    Autonomous,
    /// Permit every category after an explicit, separately confirmed user choice.
    Bypass,
}

impl PermissionMode {
    /// Returns whether unattended execution for this mode requires verified OS containment.
    #[must_use]
    pub const fn requires_sandbox(self) -> bool {
        matches!(self, Self::Autonomous | Self::Bypass)
    }
}

/// Trusted process-launch containment state, never accepted from a protocol client.
///
/// The future daemon-owned launcher derives this after preparing the OS sandbox immediately
/// before spawning the exact locked executable. It is deliberately not serializable because a
/// client-provided value would be a forgeable security claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEnforcement {
    Enforced,
    Unavailable,
}

/// A stable, provider-neutral class of requested capability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionCategory {
    Read,
    Edit,
    Command,
    Network,
    Provider,
}

/// The typed operation presented to the pure permission evaluator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    /// Stable public tool name, never a provider-native process argument or credential.
    pub tool_name: String,
    pub category: PermissionCategory,
}

/// One immutable revision of a workspace policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRecord {
    pub policy_id: String,
    pub workspace_id: String,
    pub scope: PolicyScope,
    pub revision: u64,
    /// The selected execution posture. Changing it always creates a new revision.
    #[serde(default)]
    pub mode: PermissionMode,
    /// Canonically sorted exact tool approvals. Credentials and endpoint data are never policy.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Canonically sorted broad approvals. These are intentionally typed rather than globbed.
    #[serde(default)]
    pub allowed_categories: Vec<PermissionCategory>,
}

#[cfg(test)]
mod tests {
    use super::{
        PermissionCategory, PermissionMode, PolicyRecord, PolicyScope, SandboxEnforcement,
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
        assert_eq!(policy.mode, PermissionMode::Default);
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
    fn broad_modes_require_a_daemon_verified_sandbox() {
        assert!(PermissionMode::Autonomous.requires_sandbox());
        assert!(PermissionMode::Bypass.requires_sandbox());
        assert!(!PermissionMode::Plan.requires_sandbox());
        let _ = SandboxEnforcement::Enforced;
    }
}
