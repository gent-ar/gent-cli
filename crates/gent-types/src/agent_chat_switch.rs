//! Durable provider-selection transitions for a provider-neutral chat conversation.

use crate::{
    AgentChatConversationId, AgentChatRunId, AgentChatSelection, HostEpoch, Receipt, ReceiptId,
};

/// Immutable request to continue a conversation in a new selected child run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSelectionSwitch {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub parent_run_id: AgentChatRunId,
    pub run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
}

/// A retry-stable child run and the immutable history boundary it inherited.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSelectionSwitched {
    pub receipt: Receipt,
    pub conversation_id: AgentChatConversationId,
    pub parent_run_id: AgentChatRunId,
    pub run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
    pub context_through_ordinal: u64,
}
