//! Adapter joining durable conversation prompts to the public persistence port.

use gent_ports::{
    ConversationContentReader, ConversationPromptLedger, ConversationPromptSave, LedgerError,
};
use gent_types::{ConversationContentPage, ConversationMessage, ConversationPrompt};

use super::{SqliteLedger, conversation_prompts};

impl ConversationPromptLedger for SqliteLedger {
    fn save_conversation_prompt(
        &self,
        prompt: &ConversationPrompt,
    ) -> Result<ConversationPromptSave, LedgerError> {
        conversation_prompts::save(self, prompt)
    }

    fn find_conversation_message(
        &self,
        message_id: &str,
    ) -> Result<Option<ConversationMessage>, LedgerError> {
        conversation_prompts::find(self, message_id)
    }

    fn list_run_messages(&self, run_id: &str) -> Result<Vec<ConversationMessage>, LedgerError> {
        conversation_prompts::list(self, run_id)
    }
}

impl ConversationContentReader for SqliteLedger {
    fn read_conversation_content(
        &self,
        conversation_id: &str,
        before_ordinal: Option<u64>,
        limit: u16,
    ) -> Result<ConversationContentPage, LedgerError> {
        conversation_prompts::content(self, conversation_id, before_ordinal, limit)
    }
}
