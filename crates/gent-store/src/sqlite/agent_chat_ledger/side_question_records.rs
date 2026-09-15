use gent_ports::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatSideQuestionRecord, AgentChatSideQuestionStatus,
};
use rusqlite::OptionalExtension;

use super::super::super::queries::storage_error;

pub(super) fn find(
    connection: &rusqlite::Connection,
    side_question_id: &str,
) -> Result<Option<AgentChatSideQuestionRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT side_question_id, conversation_id, question, status, answer, failure_reason, created_at_unix_ms FROM agent_chat_side_questions WHERE side_question_id = ?1",
            [side_question_id],
            row_to_record,
        )
        .optional()
        .map_err(storage_error)?
        .transpose()
}

pub(super) fn row_to_record(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<AgentChatSideQuestionRecord, LedgerError>> {
    let side_question_id: String = row.get(0)?;
    let conversation_id: String = row.get(1)?;
    let question: String = row.get(2)?;
    let status: String = row.get(3)?;
    let answer: Option<String> = row.get(4)?;
    let failure_reason: Option<String> = row.get(5)?;
    let created_at_unix_ms: u64 = row.get(6)?;
    Ok(
        parse_status(&status).map(|status| AgentChatSideQuestionRecord {
            side_question_id,
            conversation_id: AgentChatConversationId(conversation_id),
            question,
            status,
            answer,
            failure_reason,
            created_at_unix_ms,
        }),
    )
}

pub(super) fn parse_status(status: &str) -> Result<AgentChatSideQuestionStatus, LedgerError> {
    match status {
        "pending" => Ok(AgentChatSideQuestionStatus::Pending),
        "answered" => Ok(AgentChatSideQuestionStatus::Answered),
        "failed" => Ok(AgentChatSideQuestionStatus::Failed),
        "cancelled" => Ok(AgentChatSideQuestionStatus::Cancelled),
        other => Err(LedgerError::Invariant(format!(
            "unknown agent chat side question status: {other}"
        ))),
    }
}
