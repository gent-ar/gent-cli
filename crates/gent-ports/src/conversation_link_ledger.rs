use gent_types::{ConversationActivityFact, ConversationLink, ConversationThreadState};

use crate::LedgerError;

pub trait ConversationLinkLedger: Send + Sync {
    fn record_conversation_link(
        &self,
        link: &ConversationLink,
        facts: &[ConversationActivityFact],
        request_identity: &str,
    ) -> Result<(), LedgerError>;

    fn append_conversation_orchestration_fact(
        &self,
        fact: &ConversationActivityFact,
        request_identity: &str,
    ) -> Result<(), LedgerError>;

    fn read_conversation_thread_state(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationThreadState>, LedgerError>;

    fn read_conversation_parent(
        &self,
        child_conversation_id: &str,
    ) -> Result<Option<ConversationLink>, LedgerError>;

    fn read_conversation_children(
        &self,
        parent_conversation_id: &str,
    ) -> Result<Vec<ConversationLink>, LedgerError>;
}
