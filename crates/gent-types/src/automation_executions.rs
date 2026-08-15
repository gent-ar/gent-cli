//! Durable automation execution identities and monotonic lifecycle phases.

use serde::{Deserialize, Serialize};

/// Monotonic lifecycle for a single accepted automation trigger.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutomationExecutionPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

impl AutomationExecutionPhase {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Interrupted)
    }
}

/// One durable trigger execution. Its trigger key prevents duplicate scheduling.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationExecutionRecord {
    pub execution_id: String,
    pub workspace_id: String,
    pub automation_id: String,
    pub trigger_key: String,
    pub phase: AutomationExecutionPhase,
}
