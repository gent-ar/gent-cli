use gent_ports::{ForgeConnectorLedger, LedgerError};
use gent_types::ForgeConnectorRecord;

use super::{SqliteLedger, forge_connectors};

impl ForgeConnectorLedger for SqliteLedger {
    fn create_forge_connector(&self, connector: &ForgeConnectorRecord) -> Result<(), LedgerError> {
        forge_connectors::create(self, connector)
    }

    fn find_forge_connector(
        &self,
        connector_id: &str,
    ) -> Result<Option<ForgeConnectorRecord>, LedgerError> {
        forge_connectors::find(self, connector_id)
    }

    fn list_forge_connectors(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<ForgeConnectorRecord>, LedgerError> {
        forge_connectors::list(self, workspace_id)
    }

    fn replace_forge_connector(&self, connector: &ForgeConnectorRecord) -> Result<(), LedgerError> {
        forge_connectors::replace(self, connector)
    }
}
