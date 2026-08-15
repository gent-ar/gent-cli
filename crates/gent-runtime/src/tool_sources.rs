//! Coordinator calls for durable, credential-free tool-source declarations.

use gent_ports::ToolSourceLedger;
use gent_types::ToolSourceRecord;

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + ToolSourceLedger,
{
    /// Persists a tool declaration without connecting to or starting its source.
    ///
    /// # Errors
    /// Returns an error when the declaration violates durable workspace invariants.
    pub fn create_tool_source(&self, source: &ToolSourceRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_tool_source(source)?)
    }

    /// Lists durable declarations without discovering, spawning, or invoking tools.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn tool_sources(&self, workspace_id: &str) -> Result<Vec<ToolSourceRecord>, RuntimeError> {
        Ok(self.ledger.list_tool_sources(workspace_id)?)
    }
}
