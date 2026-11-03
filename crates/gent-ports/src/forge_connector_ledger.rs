use gent_types::{Command, ForgeConnectorRecord, Receipt, ToolSourceRecord};

use crate::LedgerError;

pub trait ForgeConnectorLedger: Send + Sync {
    fn commit_managed_forge_registration(
        &self,
        command: &Command,
        source: &ToolSourceRecord,
        connector: &ForgeConnectorRecord,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Receipt, LedgerError>;

    fn create_forge_connector(&self, connector: &ForgeConnectorRecord) -> Result<(), LedgerError>;

    fn find_forge_connector(
        &self,
        connector_id: &str,
    ) -> Result<Option<ForgeConnectorRecord>, LedgerError>;

    fn list_forge_connectors(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<ForgeConnectorRecord>, LedgerError>;

    fn replace_forge_connector(&self, connector: &ForgeConnectorRecord) -> Result<(), LedgerError>;
}
