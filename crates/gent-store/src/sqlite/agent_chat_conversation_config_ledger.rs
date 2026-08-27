//! Adapter joining immutable conversation-config queries to the public persistence port.

use gent_ports::{AgentChatConversationConfigLedger, LedgerError};
use gent_types::AgentChatConversationConfigRecord;

use super::{SqliteLedger, agent_chat_conversation_config};

impl AgentChatConversationConfigLedger for SqliteLedger {
    fn save_conversation_config(
        &self,
        config: &AgentChatConversationConfigRecord,
    ) -> Result<(), LedgerError> {
        agent_chat_conversation_config::save(self, config)
    }

    fn current_conversation_config(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AgentChatConversationConfigRecord>, LedgerError> {
        agent_chat_conversation_config::current_conversation_config(self, conversation_id)
    }
}
