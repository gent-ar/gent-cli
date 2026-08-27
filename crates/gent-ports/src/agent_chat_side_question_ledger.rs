//! Durable boundary for asking, completing, cancelling, and reading side questions.

use gent_types::{
    AgentChatConversationId, AgentChatSideQuestion, AgentChatSideQuestionAsked,
    AgentChatSideQuestionCancel, AgentChatSideQuestionCancelled, AgentChatSideQuestionOutcome,
    AgentChatSideQuestionRecord,
};

use crate::LedgerError;

/// Maximum side questions with a durable `Pending` status for one conversation at once.
pub const MAX_LIVE_SIDE_QUESTIONS_PER_CONVERSATION: u32 = 3;

/// Maximum side questions with a durable `Pending` status across every conversation at once.
pub const MAX_LIVE_SIDE_QUESTIONS_TOTAL: u32 = 8;

/// Persistence boundary for a bounded, provider-neutral question about a conversation.
pub trait AgentChatSideQuestionLedger: Send + Sync {
    /// Atomically accepts a new side question as `Pending` under a retry-stable identity,
    /// rejecting it when the source conversation is unknown or a concurrency bound is exceeded.
    ///
    /// # Errors
    /// Returns an error when the conversation is unknown, a live-question bound would be
    /// exceeded, the idempotency key is owned by another command, or the write cannot persist.
    fn ask_agent_chat_side_question(
        &self,
        ask: &AgentChatSideQuestion,
        side_question_id: &str,
    ) -> Result<AgentChatSideQuestionAsked, LedgerError>;

    /// Atomically settles a `Pending` side question with its final outcome.
    ///
    /// # Errors
    /// Returns an error when the side question is unknown or is no longer `Pending`.
    fn complete_agent_chat_side_question(
        &self,
        side_question_id: &str,
        outcome: &AgentChatSideQuestionOutcome,
    ) -> Result<AgentChatSideQuestionRecord, LedgerError>;

    /// Atomically marks a still-`Pending` side question `Cancelled` under a retry-stable
    /// identity. Does not interrupt any dispatched provider process.
    ///
    /// # Errors
    /// Returns an error when the side question is unknown, the idempotency key is owned by
    /// another command, or the write cannot persist.
    fn cancel_agent_chat_side_question(
        &self,
        cancel: &AgentChatSideQuestionCancel,
    ) -> Result<AgentChatSideQuestionCancelled, LedgerError>;

    /// Reads one durable side question by its identity.
    ///
    /// # Errors
    /// Returns an error when the read cannot complete.
    fn agent_chat_side_question(
        &self,
        side_question_id: &str,
    ) -> Result<Option<AgentChatSideQuestionRecord>, LedgerError>;

    /// Reads every durable side question belonging to one conversation, newest first.
    ///
    /// # Errors
    /// Returns an error when the read cannot complete.
    fn list_agent_chat_side_questions(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Vec<AgentChatSideQuestionRecord>, LedgerError>;
}
