//! Atomic ownership boundary for a selected child run and its frozen history boundary.

use gent_types::{AgentChatSelectionSwitch, AgentChatSelectionSwitched};

use crate::LedgerError;

/// Persists a receipt-backed provider/model switch without starting a provider.
pub trait AgentChatSelectionLedger: Send + Sync {
    /// Fences the expected current run and creates its immutable selected child atomically.
    ///
    /// # Errors
    /// Returns an error when the epoch, parent, receipt ownership, or transaction is invalid.
    fn switch_agent_chat_selection(
        &self,
        switch: &AgentChatSelectionSwitch,
    ) -> Result<AgentChatSelectionSwitched, LedgerError>;
}
