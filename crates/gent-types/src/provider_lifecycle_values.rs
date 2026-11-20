use serde::{Deserialize, Serialize};

use crate::{RootActivity, TokenUsage, WorkPhase};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunVersionLock {
    pub provider: String,
    pub canonical_path: String,
    pub file_identity: String,
    pub digest_sha256: String,
    pub version: String,
    pub compatibility_entry: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedProviderLock {
    pub run_lock: RunVersionLock,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstallProvenance {
    pub package_name: String,
    pub package_version: String,
    pub package_integrity: String,
    pub package_policy_digest_sha256: String,
    pub node_runtime_digest_sha256: String,
    pub release_artifact_digest_sha256: String,
    pub receipt_fingerprint_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedProviderInstallation {
    pub lock: ProvisionedProviderLock,
    pub provenance: ProviderInstallProvenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NormalizedProviderEvent {
    Output {
        text: String,
        is_partial: bool,
    },
    Thinking {
        text: String,
        is_partial: bool,
    },
    ToolInputDelta {
        block_index: u64,
        partial_json: String,
    },
    ToolOutputDelta {
        tool_use_id: String,
        text: String,
        is_partial: bool,
    },
    TurnStarted {
        turn_id: String,
    },
    TurnEnded {
        turn_id: String,
    },
    ContextUsage {
        used_tokens: u64,
        window_tokens: Option<u64>,
    },
    TokenUsage {
        usage: TokenUsage,
    },
    PlanProposed {
        text: String,
    },
    RootActivity {
        activity: RootActivity,
    },
    ChildStarted {
        child_id: String,
        parent_tool_use_id: String,
    },
    ChildTerminal {
        child_id: String,
        phase: WorkPhase,
    },
    CommandTerminal {
        command_id: String,
        phase: WorkPhase,
    },
    DecisionSettled {
        decision_id: String,
    },
    ProviderFailure {
        classification: ProviderFailureClassification,
        message: String,
        #[serde(default, rename = "resetsAt", skip_serializing_if = "Option::is_none")]
        resets_at: Option<u64>,
    },
    TransportDiagnostic {
        classification: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderFailureClassification {
    Authentication,
    RateLimited,
    ContextLimit,
    OutputLimit,
    Overloaded,
    Provider,
    Lifecycle,
    SessionUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEvent {
    Output { text: String },
    DecisionSettled { decision_id: String },
    Terminal { reason: String },
}

#[cfg(test)]
#[path = "provider_lifecycle_values_tests.rs"]
mod tests;
