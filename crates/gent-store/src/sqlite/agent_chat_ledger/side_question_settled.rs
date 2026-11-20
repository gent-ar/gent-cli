use gent_ports::LedgerError;
use gent_types::{
    AgentChatSideQuestionRecord, AgentChatSideQuestionStatus, ConversationActivityFact,
    ConversationActivityScope,
};
use rusqlite::Transaction;

use super::super::super::{
    conversation_link_ledger,
    queries::{host_ingress, storage_error},
};

pub(super) fn publish_settled(
    transaction: &Transaction<'_>,
    record: &AgentChatSideQuestionRecord,
) -> Result<(), LedgerError> {
    if record.status == AgentChatSideQuestionStatus::Pending {
        return Ok(());
    }
    let Some(thread) =
        conversation_link_ledger::thread_state(transaction, &record.conversation_id.0)?
    else {
        return Ok(());
    };
    let fact = ConversationActivityFact::SideQuestionSettled {
        scope: ConversationActivityScope {
            turn_id: thread.fact_turn_id(),
            conversation_id: thread.conversation_id,
            run_id: thread.run_id,
            host_epoch: host_ingress(transaction)?.epoch,
            cursor: 0,
        },
        side_question_id: record.side_question_id.clone(),
        ask_receipt_id: ask_receipt_id(transaction, &record.side_question_id)?,
        status: record.status,
        answer: record.answer.clone(),
        failure_code: record.failure_code,
        failure_reason: record.failure_reason.clone(),
    };
    conversation_link_ledger::publish(
        transaction,
        &fact,
        &format!("side-question-settled:{}", record.side_question_id),
    )
}

fn ask_receipt_id(
    transaction: &Transaction<'_>,
    side_question_id: &str,
) -> Result<String, LedgerError> {
    transaction
        .query_row(
            "SELECT r.receipt_id FROM agent_chat_side_questions q JOIN receipts r ON r.idempotency_key = q.idempotency_key WHERE q.side_question_id = ?1",
            [side_question_id],
            |row| row.get(0),
        )
        .map_err(storage_error)
}
