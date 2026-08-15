//! Adapter joining immutable tool-source declarations to the public persistence port.

use gent_ports::{LedgerError, ToolSourceLedger};
use gent_types::ToolSourceRecord;

use super::{SqliteLedger, tool_sources};

impl ToolSourceLedger for SqliteLedger {
    fn create_tool_source(&self, source: &ToolSourceRecord) -> Result<(), LedgerError> {
        tool_sources::create(self, source)
    }

    fn list_tool_sources(&self, workspace_id: &str) -> Result<Vec<ToolSourceRecord>, LedgerError> {
        tool_sources::list(self, workspace_id)
    }
}
