use gent_types::{AgentChatConversationId, AgentChatProjectionPage, AgentChatProjectionTail};

use crate::LedgerError;

pub trait AgentChatProjectionLedger: Send + Sync {
    fn agent_chat_projection_page(
        &self,
        conversation_id: &AgentChatConversationId,
        after_cursor: u64,
        limit: u16,
    ) -> Result<AgentChatProjectionPage, LedgerError>;

    fn agent_chat_projection_tail(
        &self,
        conversation_id: &AgentChatConversationId,
        transcript_limit: u16,
        activity_limit: u16,
    ) -> Result<AgentChatProjectionTail, LedgerError>;
}
