//! Durable conversation and turn identities shared across runtime boundaries.

use serde::{Deserialize, Serialize};

use crate::RunLiveStatus;

/// Immutable conversation identity. Content and presentation metadata live in separate domains.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRecord {
    pub conversation_id: String,
}

/// Durable lifecycle state for one user-visible turn.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DurableTurnPhase {
    Active,
    WaitingPermission,
    WaitingQuestion,
    Completed,
    Interrupted,
    Failed,
}

impl DurableTurnPhase {
    /// Whether the phase has no valid successor.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Interrupted | Self::Failed)
    }
}

/// An immutable turn identity plus its monotonic durable state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    pub turn_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub sequence: u64,
    pub phase: DurableTurnPhase,
}

/// One immutable run in a read-only conversation status response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRunStatus {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub provider: String,
    pub active_turn_id: Option<String>,
    pub live_status: Option<RunLiveStatus>,
}

/// Read-only durable conversation hierarchy with optional lifecycle projections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStatus {
    pub conversation_id: String,
    pub runs: Vec<ConversationRunStatus>,
}

/// Non-content provenance for one generated conversation artifact.
///
/// The artifact text is intentionally excluded so timeline reads cannot expose a transcript.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationArtifactSummary {
    pub artifact_id: String,
    pub kind: crate::ConversationArtifactKind,
    pub source_turn_ids: Vec<String>,
    pub provider: String,
    pub model_version: String,
    pub input_digest: String,
    pub status: crate::ConversationArtifactStatus,
    pub supersedes_artifact_id: Option<String>,
}

/// Immutable run lineage and ordered turn lifecycle states for one conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelineRun {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub provider: String,
    pub turns: Vec<TurnRecord>,
}

/// Read-only, non-content timeline suitable for conversation and session UIs.
///
/// Provider-native session identifiers and all user or provider text remain private.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimeline {
    pub conversation_id: String,
    pub runs: Vec<ConversationTimelineRun>,
    pub artifacts: Vec<ConversationArtifactSummary>,
}
