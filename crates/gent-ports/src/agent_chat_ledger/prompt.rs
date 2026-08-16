//! Atomic ownership boundary for one immutable agent-chat prompt.

use gent_types::{AgentChatPromptCreate, AgentChatPromptSaved};

use crate::LedgerError;

/// Persists a user prompt, its receipt, turn, message, and transcript ordinal in one transaction.
pub trait AgentChatPromptLedger: Send + Sync {
    /// Resolves the durable current run for the supplied conversation and saves one prompt.
    ///
    /// The same request correlation returns the originally settled receipt and message. A changed
    /// receipt, conversation, disposition, or text is a durable ownership conflict.
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, hierarchy is unknown, or the
    /// complete transaction cannot be persisted.
    fn save_agent_chat_prompt(
        &self,
        prompt: &AgentChatPromptCreate,
    ) -> Result<AgentChatPromptSaved, LedgerError>;
}
