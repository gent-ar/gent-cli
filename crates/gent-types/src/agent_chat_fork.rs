//! Typed request/result for copying a conversation's durable prompt history into a new one.

use crate::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, HostEpoch, Receipt, ReceiptId,
};

/// Client correlation and source boundary required to fork one durable conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatFork {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub source_conversation_id: AgentChatConversationId,
    /// Copies every message up to and including this one.
    pub fork_through_message_id: String,
}

/// A retry-stable new conversation seeded from a source conversation's prior messages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatForked {
    pub receipt: Receipt,
    pub source_conversation_id: AgentChatConversationId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
}
