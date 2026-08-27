//! Durable MCP connector identities and monotonic lifecycle phases.

use serde::{Deserialize, Serialize};

/// Monotonic lifecycle for one explicitly requested MCP connection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpConnectorPhase {
    Requested,
    Connecting,
    Ready,
    Failed,
    Interrupted,
}

impl McpConnectorPhase {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Failed | Self::Interrupted)
    }
}

/// One durable lifecycle record for a credential-free MCP source declaration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectorRecord {
    pub connector_id: String,
    pub workspace_id: String,
    pub tool_source_id: String,
    pub phase: McpConnectorPhase,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeConnectorRecord {
    pub connector_id: String,
    pub workspace_id: String,
    pub tool_source_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub phase: McpConnectorPhase,
    pub declared_tools: Vec<String>,
    pub discovered_tools: Vec<String>,
    pub enabled: bool,
}
