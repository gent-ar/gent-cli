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

/// A typed tool fact with a bounded operation preview, never raw provider output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub tool_use_id: String,
    pub tool_name: String,
    pub phase: ToolPhase,
    /// Presentation-only arguments produced by [`operation_preview`]. Never execute this
    /// value: it may contain redactions and explicit truncation markers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Optional digest permits result correlation without retaining provider output.
    pub output_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
}

#[path = "tool_activity_preview.rs"]
mod preview;
pub use preview::{operation_preview, operation_preview_with_limit};
