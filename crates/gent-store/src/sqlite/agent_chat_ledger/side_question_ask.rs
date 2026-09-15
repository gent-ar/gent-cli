use gent_ports::{
    LedgerError, MAX_LIVE_SIDE_QUESTIONS_PER_CONVERSATION, MAX_LIVE_SIDE_QUESTIONS_TOTAL,
};
use gent_types::{
    AgentChatConversationId, AgentChatSideQuestion, AgentChatSideQuestionAsked,
    AgentChatSideQuestionRecord, AgentChatSideQuestionStatus, Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::super::SqliteLedger;
use super::super::super::queries::{
    find_receipt, insert_receipt, receipt_matches_command, storage_error,
};
use super::super::prompt_dispatch::require_open;
use super::records::parse_status;
use super::{conflict, idempotency_key};

pub(super) fn persist_ask(
    ledger: &SqliteLedger,
    ask: &AgentChatSideQuestion,
    side_question_id: &str,
) -> Result<AgentChatSideQuestionAsked, LedgerError> {
    validate_ask(ask, side_question_id)?;
    let idempotency_key =
        idempotency_key("gent-agent-chat-side-question-ask-v1", &ask.request_id.0);
    let command = ask_command(ask, side_question_id, &idempotency_key);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, ask.host_epoch)?;
    if let Some(existing) = existing_ask(&transaction, &idempotency_key, ask, side_question_id)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(conflict("ask"));
        }
        return Ok(existing);
    }
    if find_receipt(&transaction, &idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat side question idempotency key is owned by another command".into(),
        ));
    }
    if !conversation_exists(&transaction, &ask.conversation_id.0)? {
        return Err(LedgerError::Invariant(
            "agent chat side question conversation is unknown".into(),
        ));
    }
    enforce_live_bounds(&transaction, &ask.conversation_id.0)?;
    let receipt = Receipt {
        receipt_id: ask.receipt_id.clone(),
        idempotency_key: idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: ask.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_side_questions (side_question_id, idempotency_key, conversation_id, question, status, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![
                side_question_id,
                idempotency_key,
                ask.conversation_id.0,
                ask.question,
                ask.created_at_unix_ms,
            ],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatSideQuestionAsked {
        receipt,
        record: AgentChatSideQuestionRecord {
            side_question_id: side_question_id.to_owned(),
            conversation_id: ask.conversation_id.clone(),
            question: ask.question.clone(),
            status: AgentChatSideQuestionStatus::Pending,
            answer: None,
            failure_reason: None,
            created_at_unix_ms: ask.created_at_unix_ms,
        },
    })
}

fn validate_ask(ask: &AgentChatSideQuestion, side_question_id: &str) -> Result<(), LedgerError> {
    if side_question_id.trim().is_empty()
        || ask.receipt_id.0.trim().is_empty()
        || ask.request_id.0.trim().is_empty()
        || ask.conversation_id.0.trim().is_empty()
        || ask.question.trim().is_empty()
    {
        return Err(LedgerError::Invariant(
            "agent chat side question identities and text must be nonempty".into(),
        ));
    }
    Ok(())
}

fn conversation_exists(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<bool, LedgerError> {
    transaction
        .query_row(
            "SELECT 1 FROM agent_chat_conversations WHERE conversation_id = ?1",
            [conversation_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)
        .map(|row| row.is_some())
}

fn enforce_live_bounds(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<(), LedgerError> {
    let per_conversation: u32 = transaction
        .query_row(
            "SELECT COUNT(*) FROM agent_chat_side_questions WHERE conversation_id = ?1 AND status = 'pending'",
            [conversation_id],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if per_conversation >= MAX_LIVE_SIDE_QUESTIONS_PER_CONVERSATION {
        return Err(LedgerError::Invariant(
            "this conversation already has the maximum number of live side questions".into(),
        ));
    }
    let total: u32 = transaction
        .query_row(
            "SELECT COUNT(*) FROM agent_chat_side_questions WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if total >= MAX_LIVE_SIDE_QUESTIONS_TOTAL {
        return Err(LedgerError::Invariant(
            "Gent already has the maximum number of live side questions".into(),
        ));
    }
    Ok(())
}

fn existing_ask(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    ask: &AgentChatSideQuestion,
    side_question_id: &str,
) -> Result<Option<AgentChatSideQuestionAsked>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, q.side_question_id, q.conversation_id, q.question, q.status, q.answer, q.failure_reason, q.created_at_unix_ms FROM agent_chat_side_questions q JOIN receipts r ON r.idempotency_key = q.idempotency_key WHERE q.idempotency_key = ?1",
            [idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, u64>(9)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((
        receipt_id,
        receipt_status,
        epoch,
        existing_id,
        conversation,
        question,
        status,
        answer,
        failure_reason,
        created_at_unix_ms,
    )) = row
    else {
        return Ok(None);
    };
    if receipt_id != ask.receipt_id.0
        || existing_id != side_question_id
        || conversation != ask.conversation_id.0
        || question != ask.question
        || receipt_status != "settled"
    {
        return Err(conflict("ask"));
    }
    Ok(Some(AgentChatSideQuestionAsked {
        receipt: Receipt {
            receipt_id: ask.receipt_id.clone(),
            idempotency_key: idempotency_key.to_owned(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        record: AgentChatSideQuestionRecord {
            side_question_id: existing_id,
            conversation_id: AgentChatConversationId(conversation),
            question,
            status: parse_status(&status)?,
            answer,
            failure_reason,
            created_at_unix_ms,
        },
    }))
}

fn ask_command(
    ask: &AgentChatSideQuestion,
    side_question_id: &str,
    idempotency_key: &str,
) -> Command {
    Command {
        receipt_id: ask.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        host_epoch: ask.host_epoch,
        kind: "agentChatAskSideQuestion".into(),
        payload: json!({
            "sideQuestionId": side_question_id,
            "conversationId": ask.conversation_id.0,
            "question": ask.question,
        }),
    }
}
