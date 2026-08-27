//! Coordinator calls for durable, versioned per-conversation advanced launch configuration.

use gent_ports::AgentChatConversationConfigLedger;
use gent_types::AgentChatConversationConfigRecord;

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + AgentChatConversationConfigLedger,
{
    /// Persists the next immutable conversation-config revision.
    ///
    /// # Errors
    /// Returns an error when the config is invalid, out of sequence, or cannot persist.
    pub fn save_conversation_config(
        &self,
        config: &AgentChatConversationConfigRecord,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.save_conversation_config(config)?)
    }

    /// Reads the latest advanced launch configuration for one conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn current_conversation_config(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AgentChatConversationConfigRecord>, RuntimeError> {
        Ok(self.ledger.current_conversation_config(conversation_id)?)
    }
}
