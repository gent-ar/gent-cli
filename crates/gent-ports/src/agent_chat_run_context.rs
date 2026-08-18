//! Read-only provenance for a run's frozen provider-neutral context boundary.

use gent_types::{AgentChatConversationId, AgentChatRunContext, AgentChatRunId};

use crate::LedgerError;

/// Resolves context policy and frozen ordinal for exactly one durable agent-chat run.
///
/// Implementations must fail closed for a run that is not a root, selection child, or reviewed
/// implementation child. They must never expose provider-native session identifiers.
pub trait AgentChatRunContextReader: Send + Sync {
    /// Reads the exact immutable context boundary belonging to `run_id` in `conversation_id`.
    ///
    /// # Errors
    /// Returns an error for an unknown or mismatched hierarchy, invalid stored values, or storage
    /// failures.
    fn read_agent_chat_run_context(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatRunContext, LedgerError>;
}
