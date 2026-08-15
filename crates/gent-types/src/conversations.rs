//! Durable conversation and turn identities shared across runtime boundaries.

use serde::{Deserialize, Serialize};

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
