//! Durable boundary for user-authored conversation prompts and their turns.

use gent_types::{ConversationMessage, ConversationPrompt};

use crate::LedgerError;

/// Result of an idempotent prompt persistence attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationPromptSave {
    Created(ConversationMessage),
    Existing(ConversationMessage),
}

/// Stores prompt content separately from receipt/event payloads.
pub trait ConversationPromptLedger: Send + Sync {
    /// Atomically creates a user prompt and its active turn, or returns its exact existing record.
    ///
    /// # Errors
    /// Returns an error when hierarchy, text bounds, or durable state invariants fail.
    fn save_conversation_prompt(
        &self,
        prompt: &ConversationPrompt,
    ) -> Result<ConversationPromptSave, LedgerError>;

    /// Reads one prompt by immutable message identity.
    ///
    /// # Errors
    /// Returns an error when durable content cannot be read.
    fn find_conversation_message(
        &self,
        message_id: &str,
    ) -> Result<Option<ConversationMessage>, LedgerError>;

    /// Lists user prompts for one run in their durable turn order.
    ///
    /// # Errors
    /// Returns an error when durable content cannot be read.
    fn list_run_messages(&self, run_id: &str) -> Result<Vec<ConversationMessage>, LedgerError>;
}
