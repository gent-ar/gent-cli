//! Adapter joining MCP connector records to the public persistence port.

use gent_ports::{
    LedgerError, McpConnectorLease, McpConnectorLeaseClaim, McpConnectorLedger, McpConnectorUpdate,
};
use gent_types::{McpConnectorPhase, McpConnectorRecord};

use super::{SqliteLedger, mcp_connectors};

impl McpConnectorLedger for SqliteLedger {
    fn create_mcp_connector(&self, connector: &McpConnectorRecord) -> Result<(), LedgerError> {
        mcp_connectors::create(self, connector)
    }

    fn find_mcp_connector(
        &self,
        connector_id: &str,
    ) -> Result<Option<McpConnectorRecord>, LedgerError> {
        mcp_connectors::find(self, connector_id)
    }

    fn replace_mcp_connector_phase(
        &self,
        connector_id: &str,
        expected: McpConnectorPhase,
        next: McpConnectorPhase,
    ) -> Result<McpConnectorUpdate, LedgerError> {
        mcp_connectors::replace_phase(self, connector_id, expected, next)
    }

    fn claim_mcp_connector_lease(
        &self,
        lease: &McpConnectorLease,
    ) -> Result<McpConnectorLeaseClaim, LedgerError> {
        mcp_connectors::claim_lease(self, lease)
    }

    fn find_mcp_connector_lease(
        &self,
        tool_source_id: &str,
    ) -> Result<Option<McpConnectorLease>, LedgerError> {
        mcp_connectors::find_lease(self, tool_source_id)
    }
}
