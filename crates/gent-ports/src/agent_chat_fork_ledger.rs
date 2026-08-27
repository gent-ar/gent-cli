//! Durable boundary for copying a conversation's prior messages into a new conversation.

use gent_types::{AgentChatConversationId, AgentChatFork, AgentChatForked, AgentChatRunId};

use crate::LedgerError;

/// Persistence boundary for forking a durable conversation's message history.
pub trait AgentChatForkLedger: Send + Sync {
    /// Atomically creates a new conversation seeded from the source conversation's messages up
    /// to and including `fork.fork_through_message_id`, under the given retry-stable identities.
    ///
    /// # Errors
    /// Returns an error when the fork point does not belong to the source conversation, the
    /// idempotency key is owned by another command, or the write cannot persist.
    fn fork_agent_chat_conversation(
        &self,
        fork: &AgentChatFork,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatForked, LedgerError>;
}
