//! Durable boundary for MCP connector lifecycle records and exclusive leases.

use gent_types::{HostEpoch, McpConnectorPhase, McpConnectorRecord};

use crate::LedgerError;

/// Durable ownership preventing two coordinators from connecting one source concurrently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConnectorLease {
    pub tool_source_id: String,
    pub lease_token: String,
    pub host_epoch: HostEpoch,
}

/// Result of atomically claiming an MCP connector lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpConnectorLeaseClaim {
    Acquired(McpConnectorLease),
    Contended(McpConnectorLease),
    Recovered {
        previous: McpConnectorLease,
        current: McpConnectorLease,
    },
}

/// Result of an optimistic MCP connector phase update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpConnectorUpdate {
    Applied(McpConnectorRecord),
    Current(McpConnectorRecord),
}

/// Persistence boundary for connector coordination; connection I/O does not belong here.
pub trait McpConnectorLedger: Send + Sync {
    /// Creates a requested connector linked to an immutable MCP source declaration.
    ///
    /// # Errors
    /// Returns an error when source or connector invariants fail.
    fn create_mcp_connector(&self, connector: &McpConnectorRecord) -> Result<(), LedgerError>;

    /// Reads a connector record by its immutable identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_mcp_connector(
        &self,
        connector_id: &str,
    ) -> Result<Option<McpConnectorRecord>, LedgerError>;

    /// Updates a connector phase only if it still equals `expected`.
    ///
    /// # Errors
    /// Returns an error when the connector is unknown or persistence fails.
    fn replace_mcp_connector_phase(
        &self,
        connector_id: &str,
        expected: McpConnectorPhase,
        next: McpConnectorPhase,
    ) -> Result<McpConnectorUpdate, LedgerError>;

    /// Claims exclusive ownership of an MCP source at the current host epoch.
    ///
    /// # Errors
    /// Returns an error when the epoch is stale or persistence fails.
    fn claim_mcp_connector_lease(
        &self,
        lease: &McpConnectorLease,
    ) -> Result<McpConnectorLeaseClaim, LedgerError>;

    /// Reads the current exclusive source lease, if any.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_mcp_connector_lease(
        &self,
        tool_source_id: &str,
    ) -> Result<Option<McpConnectorLease>, LedgerError>;
}
