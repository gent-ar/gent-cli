use gent_types::{AgentChatConversationId, HostEpoch, Receipt, ReceiptId};

use crate::LedgerError;

pub trait AgentChatArchiveLedger: Send + Sync {
    fn archive_agent_chat_conversation(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Receipt, LedgerError>;
}
