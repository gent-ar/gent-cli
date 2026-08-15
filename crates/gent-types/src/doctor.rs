//! Read-only onboarding facts projected by `gent doctor`.

use serde::{Deserialize, Serialize};

/// Canonical identity observed for a public executable without starting a session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableIdentity {
    pub canonical_path: String,
    pub file_identity: String,
    pub digest_sha256: String,
    pub version: Option<String>,
}

/// Compatibility evidence state for a public executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatibilityTrust {
    /// No signed compatibility manifest has been configured for this host.
    NotConfigured,
    /// A configured manifest verified the executable version.
    Verified,
    /// A configured manifest cannot be trusted for the observed executable.
    Untrusted,
}

/// Read-only status for one publicly installable provider.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProviderStatus {
    pub provider: String,
    pub executable: Option<ExecutableIdentity>,
    pub compatibility: CompatibilityTrust,
    pub remediation: String,
}

/// The only MCP permission state in the observer runtime milestone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpPermissionStatus {
    HardDisabledObserver,
}

/// MCP readiness facts. These never inspect or start a connector.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDoctorStatus {
    pub permission: McpPermissionStatus,
    pub remediation: String,
}

/// Opaque private integration state; it intentionally conveys no configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivateBridgeAvailability {
    NotConfigured,
    Available,
}

/// One non-mutating, user-controlled action suggested by a doctor report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorNextAction {
    pub id: String,
    pub instruction: String,
}

impl crate::DoctorReport {
    /// Builds a deliberately non-actionable report for protocol fakes and tests.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            dependencies: Vec::new(),
            public_providers: Vec::new(),
            mcp: McpDoctorStatus {
                permission: McpPermissionStatus::HardDisabledObserver,
                remediation: "MCP is hard-disabled in observer mode.".into(),
            },
            private_bridge: PrivateBridgeAvailability::NotConfigured,
            next_action: DoctorNextAction {
                id: "none".into(),
                instruction: "No action is available in this test report.".into(),
            },
        }
    }
}
