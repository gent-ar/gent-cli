//! Typed boundary for a future daemon-owned MCP connector implementation.

/// Credential-free MCP declaration selected by the runtime, never a client command or endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConnectOperation {
    pub tool_source_id: String,
    pub source_name: String,
    pub declared_tools: Vec<String>,
}

/// Content-safe result from a successfully established connector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpConnectionSummary {
    pub tool_count: u32,
}

/// Controlled failures from a future connector implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum McpConnectorError {
    #[error("MCP connector is unavailable")]
    Unavailable,
    #[error("MCP connector handshake failed")]
    HandshakeFailed,
}

/// Establishes one already-authorized MCP connection without accepting endpoint or spawn input.
pub trait McpConnectorExecutor: Send + Sync {
    /// Connects the daemon-selected declaration.
    ///
    /// # Errors
    /// Returns a controlled failure without exposing connection configuration or credentials.
    fn connect(
        &self,
        operation: &McpConnectOperation,
    ) -> Result<McpConnectionSummary, McpConnectorError>;
}
