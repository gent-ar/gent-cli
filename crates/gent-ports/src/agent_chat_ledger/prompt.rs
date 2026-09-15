//! Atomic ownership boundary for one immutable agent-chat prompt.

use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptOrigin, AgentChatPromptSaved,
    AgentChatRunId, HostEpoch, Receipt, ReceiptId,
};

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

    /// Saves a prompt only if the conversation still selects the reviewed run.
    ///
    /// The default fails closed so existing read/persistence fakes cannot accidentally claim
    /// they provide the atomic provider-readiness fence.
    ///
    /// # Errors
    /// Returns when the implementation cannot atomically confirm the expected current run.
    fn save_agent_chat_prompt_with_origin(
        &self,
        _: &AgentChatPromptCreate,
        _: &AgentChatPromptOrigin,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt origin is unavailable".into(),
        ))
    }

    fn save_agent_chat_prompt_for_run(
        &self,
        _: &AgentChatPromptCreate,
        _: &AgentChatRunId,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt run fence is unavailable".into(),
        ))
    }
}

pub trait AgentChatQueuedPromptLedger: Send + Sync {
    fn cancel_queued_agent_chat_prompt(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
    ) -> Result<Receipt, LedgerError>;

    fn steer_queued_agent_chat_prompt(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
    ) -> Result<(Receipt, AgentChatRunId), LedgerError>;

    fn interrupt_active_turn_for_steer(
        &self,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<bool, LedgerError>;
}
