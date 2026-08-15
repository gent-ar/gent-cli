//! Content-safe, provider-neutral facts about a tool invocation.

use serde::{Deserialize, Serialize};

/// Stable presentation category selected by Gent policy, never a provider claim.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolCategory {
    File,
    Shell,
    Search,
    Network,
    #[default]
    Other,
}

/// Progress phase for one tool invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPhase {
    Started,
    WaitingPermission,
    Completed,
    Failed,
}

/// A typed tool fact that excludes tool input and output content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub tool_use_id: String,
    pub tool_name: String,
    pub phase: ToolPhase,
    /// Optional digest permits result correlation without retaining provider output.
    pub output_digest: Option<String>,
}
