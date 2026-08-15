//! Provider-neutral lifecycle values used by durable projection and status transport.

use serde::{Deserialize, Serialize};

/// Durable root-turn state. Detached work never changes this state to completed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnPhase {
    Processing,
    WaitingPermission,
    WaitingQuestion,
    Compacting,
    Ready,
    Interrupted,
    Dead,
    Failed,
}

/// Explicit root activity fact. It is independent from durable turn phase.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RootActivity {
    Generating,
    Waiting,
    #[default]
    Idle,
}

impl RootActivity {
    #[must_use]
    pub const fn is_generating(self) -> bool {
        matches!(self, Self::Generating)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkPhase {
    Pending,
    Running,
    WaitingPermission,
    Done,
    Failed,
    Interrupted,
}

impl WorkPhase {
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Running | Self::WaitingPermission
        )
    }
}

/// A complete volatile snapshot sent over status transport, never transcript content.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // The wire contract intentionally transports independent UI facts.
pub struct ConversationLiveStatus {
    pub snapshot_cursor: u64,
    pub is_processing: bool,
    pub is_waiting_for_subagents: bool,
    pub has_live_subagent_work: bool,
    pub is_waiting_for_command: bool,
    pub has_live_command_work: bool,
    pub needs_attention: bool,
    pub has_error: bool,
}
