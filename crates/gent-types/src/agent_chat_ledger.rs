//! Durable, provider-neutral values for creating an agent-chat conversation.

use crate::{
    AgentChatConversationId, AgentChatRunId, AgentChatSelection, HostEpoch, Receipt, ReceiptId,
};

/// The complete durable input for a newly created agent-chat conversation.
///
/// The authority runtime generates the public conversation and run identities, and supplies an
/// idempotency key. The storage adapter must never derive either identity from prompt content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatConversationCreate {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
}

/// The one durable result of creating a conversation, including retry-safe receipt ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatConversationCreated {
    pub receipt: Receipt,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
}

#[cfg(test)]
mod tests {
    use super::{AgentChatConversationCreate, AgentChatConversationCreated};
    use crate::{
        AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
        AgentChatSelection, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
    };

    #[test]
    fn create_values_keep_authority_generated_identities_explicit() {
        let create = AgentChatConversationCreate {
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "create-1".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
        };
        let result = AgentChatConversationCreated {
            receipt: Receipt {
                receipt_id: create.receipt_id.clone(),
                idempotency_key: create.idempotency_key.clone(),
                status: ReceiptStatus::Settled,
                host_epoch: create.host_epoch,
            },
            conversation_id: create.conversation_id,
            run_id: create.run_id,
        };
        assert_eq!(result.receipt.status, ReceiptStatus::Settled);
    }
}
