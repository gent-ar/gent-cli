//! Durable boundary for immutable workspace tool-source declarations.

use gent_types::ToolSourceRecord;

use crate::LedgerError;

/// Persistence boundary for credential-free tool-source declarations.
pub trait ToolSourceLedger: Send + Sync {
    /// Saves an immutable declaration under an existing workspace.
    ///
    /// # Errors
    /// Returns an error when the declaration is invalid, conflicts, or cannot persist.
    fn create_tool_source(&self, source: &ToolSourceRecord) -> Result<(), LedgerError>;

    /// Reads one immutable source declaration by identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_tool_source(
        &self,
        tool_source_id: &str,
    ) -> Result<Option<ToolSourceRecord>, LedgerError> {
        let _ = tool_source_id;
        Err(LedgerError::Invariant(
            "tool-source lookup is unavailable".into(),
        ))
    }

    /// Lists all immutable source declarations for one workspace in creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_tool_sources(&self, workspace_id: &str) -> Result<Vec<ToolSourceRecord>, LedgerError>;
}
