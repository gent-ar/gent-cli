//! Atomic `SQLite` persistence for asking, completing, cancelling, and reading side questions.

use gent_ports::{AgentChatSideQuestionLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatSideQuestion, AgentChatSideQuestionAsked,
    AgentChatSideQuestionCancel, AgentChatSideQuestionCancelled, AgentChatSideQuestionOutcome,
    AgentChatSideQuestionRecord,
};
use sha2::{Digest, Sha256};

use super::super::SqliteLedger;
use super::super::queries::storage_error;

#[path = "side_question_ask.rs"]
mod ask;
#[path = "side_question_cancel.rs"]
mod cancel;
#[path = "side_question_records.rs"]
mod records;
use ask::persist_ask;
use cancel::{persist_cancel, persist_completion};
use records::{find, row_to_record};

impl AgentChatSideQuestionLedger for SqliteLedger {
    fn ask_agent_chat_side_question(
        &self,
        ask: &AgentChatSideQuestion,
        side_question_id: &str,
    ) -> Result<AgentChatSideQuestionAsked, LedgerError> {
        persist_ask(self, ask, side_question_id)
    }

    fn complete_agent_chat_side_question(
        &self,
        side_question_id: &str,
        outcome: &AgentChatSideQuestionOutcome,
    ) -> Result<AgentChatSideQuestionRecord, LedgerError> {
        persist_completion(self, side_question_id, outcome)
    }

    fn cancel_agent_chat_side_question(
        &self,
        cancel: &AgentChatSideQuestionCancel,
    ) -> Result<AgentChatSideQuestionCancelled, LedgerError> {
        persist_cancel(self, cancel)
    }

    fn agent_chat_side_question(
        &self,
        side_question_id: &str,
    ) -> Result<Option<AgentChatSideQuestionRecord>, LedgerError> {
        let connection = self.lock()?;
        find(&connection, side_question_id)
    }

    fn list_agent_chat_side_questions(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Vec<AgentChatSideQuestionRecord>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT side_question_id, conversation_id, question, status, answer, failure_reason, created_at_unix_ms FROM agent_chat_side_questions WHERE conversation_id = ?1 ORDER BY created_at_unix_ms DESC, side_question_id DESC",
            )
            .map_err(storage_error)?;
        statement
            .query_map([&conversation_id.0], row_to_record)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
    }
}

fn conflict(action: &str) -> LedgerError {
    LedgerError::Invariant(format!(
        "agent chat side question {action} retry conflicts with durable ownership"
    ))
}

fn idempotency_key(namespace: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("{:x}", digest.finalize())
}
