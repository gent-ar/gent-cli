//! Read-only terminal settlement derivation for a durably settled provider-bound turn.

use gent_types::AgentChatTerminalSettlement;

use crate::LedgerError;

/// Reads a terminal settlement only when dispatch settlement and turn terminality agree.
///
/// A missing result means the requested tuple is unknown or has not settled durably. Callers
/// must never infer settlement from a provider process, client state, or transcript text.
pub trait AgentChatTerminalSettlementReader: Send + Sync {
    /// Reads the latest durable terminal settlement for one exact run and turn.
    ///
    /// # Errors
    /// Returns an error only when persistence cannot be read or an invalid persisted state occurs.
    fn read_agent_chat_terminal_settlement(
        &self,
        run_id: &str,
        turn_id: &str,
    ) -> Result<Option<AgentChatTerminalSettlement>, LedgerError>;
}
