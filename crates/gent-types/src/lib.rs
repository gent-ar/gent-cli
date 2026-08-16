//! Stable value types shared by every public Gent crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

mod attachments;
mod automation_executions;
mod capability_catalog;
mod conversation_activity;
mod conversation_activity_record;
mod conversation_artifact;
mod conversation_content;
mod conversation_prompts;
mod conversations;
mod decision;
mod doctor;
mod event_resume;
mod external_provider_bridge;
mod git_operations;
mod lifecycle_signal;
mod lifecycle_state;
mod mcp_connectors;
mod observer_tap;
mod onboarding;
mod policies;
mod run_checkpoints;
mod run_projection;
mod runtime_update;
mod tool_activity;
mod tool_sources;
mod workspaces;

pub use attachments::{
    AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, TurnAttachment,
};
pub use automation_executions::{AutomationExecutionPhase, AutomationExecutionRecord};
pub use capability_catalog::CapabilityCatalogRecord;
pub use conversation_activity::{
    ActivityWork, ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivity,
    ConversationActivityFact, ConversationActivityScope, ConversationActivityState,
};
pub use conversation_activity_record::ConversationActivityRecord;
pub use conversation_artifact::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
};
pub use conversation_content::{
    ConversationContentCursor, ConversationContentCursorError, ConversationContentEntry,
    ConversationContentPage,
};
pub use conversation_prompts::{ConversationMessage, ConversationPrompt};
pub use conversations::{
    ConversationArtifactSummary, ConversationListItem, ConversationRecord, ConversationRunStatus,
    ConversationStatus, ConversationTimeline, ConversationTimelineRun, DurableTurnPhase,
    TurnRecord,
};
pub use decision::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};
pub use doctor::{
    CompatibilityTrust, DoctorNextAction, ExecutableIdentity, McpDoctorStatus, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};
pub use event_resume::{EventResume, EventSnapshot};
pub use external_provider_bridge::{ExternalProviderSession, ExternalProviderTerminal};
pub use git_operations::{GitOperationKind, GitOperationPhase, GitOperationRecord};
pub use lifecycle_signal::NormalizedLifecycleSignal;
pub use lifecycle_state::{ConversationLiveStatus, RootActivity, TurnPhase, WorkPhase};
pub use mcp_connectors::{McpConnectorPhase, McpConnectorRecord};
pub use observer_tap::{LegacyLifecycleTap, ObserverDiagnostic, ObserverDiagnosticCode};
pub use onboarding::{OnboardingBranch, OnboardingProvider, OnboardingReadiness, OnboardingState};
pub use policies::{PolicyRecord, PolicyScope};
pub use run_checkpoints::RunCheckpointRecord;
pub use run_projection::{RunLifecycleProjection, RunLiveStatus, RunProjectionRecord};
pub use runtime_update::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeStagingReceipt, RuntimeUpdateCandidate,
    RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState,
    RuntimeUpdateFailure, RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus,
    RuntimeVersion, SignedRuntimeRelease,
};
pub use tool_activity::{ToolActivity, ToolCategory, ToolPhase};
pub use tool_sources::{ToolSourceKind, ToolSourceRecord};
pub use workspaces::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

pub const PROTOCOL_MIN: u16 = 1;
pub const PROTOCOL_MAX: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HostEpoch(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);

impl ReceiptId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for ReceiptId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptStatus {
    Accepted,
    Settled,
    Unprovable,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub status: ReceiptStatus,
    pub host_epoch: HostEpoch,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub cursor: u64,
    pub event_id: String,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub kind: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitySet(pub Vec<String>);

impl CapabilitySet {
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let mut shared = self
            .0
            .iter()
            .filter(|capability| other.0.contains(*capability))
            .cloned()
            .collect::<Vec<_>>();
        shared.sort();
        shared.dedup();
        Self(shared)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_epoch: HostEpoch,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub name: String,
    pub present: bool,
    pub version: Option<String>,
    pub remediation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub dependencies: Vec<DependencyStatus>,
    pub public_providers: Vec<PublicProviderStatus>,
    pub mcp: McpDoctorStatus,
    pub private_bridge: PrivateBridgeAvailability,
    pub next_action: DoctorNextAction,
}

/// Immutable provenance captured before a public-provider process is allowed to start.
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

/// Provider-neutral events suitable for persistence and client projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NormalizedProviderEvent {
    Output {
        text: String,
    },
    TurnStarted {
        turn_id: String,
    },
    TurnEnded {
        turn_id: String,
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
    TransportDiagnostic {
        classification: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEvent {
    Output { text: String },
    DecisionSettled { decision_id: String },
    Terminal { reason: String },
}

#[cfg(test)]
mod tests {
    use super::CapabilitySet;

    #[test]
    fn capability_intersection_is_sorted_and_unique() {
        let left = CapabilitySet(vec!["events".into(), "receipts".into(), "events".into()]);
        let right = CapabilitySet(vec!["events".into(), "status".into()]);
        assert_eq!(
            left.intersection(&right),
            CapabilitySet(vec!["events".into()])
        );
    }
}
