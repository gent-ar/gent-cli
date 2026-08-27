use gent_types::ForgeConnectorRecord;

use crate::LedgerError;

pub trait ForgeConnectorLedger: Send + Sync {
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
