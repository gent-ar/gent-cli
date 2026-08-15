//! Durable, credential-free declarations of tool providers available to a workspace.

use serde::{Deserialize, Serialize};

/// The boundary that supplies a declared set of tools.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolSourceKind {
    McpServer,
    BuiltIn,
    HostIntegration,
}

/// An immutable tool-source declaration. Connection settings and credentials do not belong here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSourceRecord {
    pub tool_source_id: String,
    pub workspace_id: String,
    pub kind: ToolSourceKind,
    /// Stable human-readable source identity, never an endpoint or a secret.
    pub source_name: String,
    /// Canonically sorted, unique qualified tool names exposed by this declaration.
    pub declared_tools: Vec<String>,
}
