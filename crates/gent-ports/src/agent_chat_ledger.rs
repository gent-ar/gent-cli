//! Atomic ownership boundary for creating a provider-neutral agent-chat conversation.

use gent_types::{AgentChatConversationCreate, AgentChatConversationCreated};

use crate::LedgerError;

mod prompt;
pub use prompt::AgentChatPromptLedger;

/// Durable creation boundary for an immutable conversation, root run, selection, and receipt.
pub trait AgentChatLedger: Send + Sync {
    /// Atomically checks the host fence, owns the idempotency key, and creates the hierarchy.
    ///
    /// A retry with the same complete input returns the original settled receipt and identities.
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, input ownership conflicts, or
    /// persistence fails.
    fn create_agent_chat_conversation(
        &self,
        create: &AgentChatConversationCreate,
    ) -> Result<AgentChatConversationCreated, LedgerError>;
}
