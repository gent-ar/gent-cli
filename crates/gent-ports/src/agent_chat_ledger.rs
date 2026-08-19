//! Atomic ownership boundary for creating a provider-neutral agent-chat conversation.

use gent_types::{AgentChatConversationCreate, AgentChatConversationCreated, WorkspaceRecord};

use crate::LedgerError;

mod prompt;
mod prompt_dispatch;
mod read;
mod switch;
pub use prompt::AgentChatPromptLedger;
pub use prompt_dispatch::AgentChatPromptDispatchLedger;
pub use read::AgentChatReadLedger;
pub use switch::AgentChatSelectionLedger;

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

/// Atomic conversation creation bound to one daemon-validated workspace.
///
/// Provider-capable runtime services use this boundary exclusively. The low-level create port is
/// retained for isolated ledger fixtures; an unbound conversation cannot accept prompts or run a
/// provider.
pub trait AgentChatWorkspaceLedger: Send + Sync {
    /// Atomically persists the canonical workspace and creates its bound conversation.
    ///
    /// # Errors
    /// Returns an error when durable workspace ownership conflicts or persistence fails.
    fn create_agent_chat_conversation_in_workspace(
        &self,
        create: &AgentChatConversationCreate,
        workspace: &WorkspaceRecord,
    ) -> Result<AgentChatConversationCreated, LedgerError>;

    /// Resolves the one durable workspace binding for a conversation run.
    ///
    /// # Errors
    /// Returns an error when the run is not in the conversation or lacks a workspace binding.
    fn agent_chat_workspace_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<WorkspaceRecord, LedgerError>;
}
