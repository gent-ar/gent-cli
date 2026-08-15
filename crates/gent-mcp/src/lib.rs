//! MCP boundary. The daemon does not spawn MCP processes in this milestone.

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP support is not enabled")]
    Disabled,
}
