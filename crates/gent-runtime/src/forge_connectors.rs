use gent_ports::{ForgeConnectorLedger, Ledger, LedgerError, ToolSourceLedger};
use gent_types::{ForgeConnectorRecord, ToolSourceKind};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: Ledger + ForgeConnectorLedger,
{
    pub fn forge_connectors(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<ForgeConnectorRecord>, RuntimeError> {
        Ok(self.ledger.list_forge_connectors(workspace_id)?)
    }

    pub fn forge_connector(
        &self,
        connector_id: &str,
    ) -> Result<Option<ForgeConnectorRecord>, RuntimeError> {
        Ok(self.ledger.find_forge_connector(connector_id)?)
    }

    pub fn create_forge_connector(
        &self,
        connector: &ForgeConnectorRecord,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_forge_connector(connector)?)
    }

    pub fn set_forge_connector_enabled(
        &self,
        workspace_id: &str,
        connector_id: &str,
        enabled: bool,
    ) -> Result<ForgeConnectorRecord, RuntimeError> {
        let Some(mut connector) = self.ledger.find_forge_connector(connector_id)? else {
            return Err(LedgerError::Invariant("Forge connector does not exist".into()).into());
        };
        if connector.workspace_id != workspace_id {
            return Err(LedgerError::Invariant(
                "Forge connector belongs to another workspace".into(),
            )
            .into());
        }
        connector.enabled = enabled;
        self.ledger.replace_forge_connector(&connector)?;
        Ok(connector)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForgeInvocation {
    pub connector_id: String,
    pub workspace_id: String,
    pub tool_source_id: String,
    pub tool_name: Option<String>,
}

impl<L> Coordinator<L>
where
    L: Ledger + ForgeConnectorLedger + ToolSourceLedger,
{
    pub fn forge_invocation(
        &self,
        workspace_id: &str,
        connector_id: &str,
        tool_name: Option<&str>,
    ) -> Result<ForgeInvocation, RuntimeError> {
        let connector = self
            .ledger
            .find_forge_connector(connector_id)?
            .ok_or_else(|| LedgerError::Invariant("Forge connector does not exist".into()))?;
        if connector.workspace_id != workspace_id || !connector.enabled {
            return Err(LedgerError::Invariant(
                "Forge connector is not enabled for this workspace".into(),
            )
            .into());
        }
        if !matches!(connector.phase, gent_types::McpConnectorPhase::Ready) {
            return Err(LedgerError::Invariant("Forge connector is not ready".into()).into());
        }
        let source = self
            .ledger
            .find_tool_source(&connector.tool_source_id)?
            .ok_or_else(|| LedgerError::Invariant("Forge tool source does not exist".into()))?;
        if source.workspace_id != workspace_id || source.kind != ToolSourceKind::McpServer {
            return Err(
                LedgerError::Invariant("Forge tool source is not an MCP source".into()).into(),
            );
        }
        if source.declared_tools != connector.declared_tools {
            return Err(LedgerError::Invariant("Forge tool catalog is stale".into()).into());
        }
        if let Some(tool) = tool_name {
            if !connector.discovered_tools.iter().any(|name| name == tool) {
                return Err(LedgerError::Invariant(
                    "Forge tool is not in the discovered catalog".into(),
                )
                .into());
            }
        }
        Ok(ForgeInvocation {
            connector_id: connector.connector_id,
            workspace_id: connector.workspace_id,
            tool_source_id: connector.tool_source_id,
            tool_name: tool_name.map(str::to_owned),
        })
    }
}
