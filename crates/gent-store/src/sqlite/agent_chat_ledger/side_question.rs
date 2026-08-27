//! Atomic `SQLite` persistence for asking, completing, cancelling, and reading side questions.

use gent_ports::{
    AgentChatSideQuestionLedger, IngressMode, LedgerError, MAX_LIVE_SIDE_QUESTIONS_PER_CONVERSATION,
    MAX_LIVE_SIDE_QUESTIONS_TOTAL,
};
use gent_types::{
    AgentChatConversationId, AgentChatSideQuestion, AgentChatSideQuestionAsked,
    AgentChatSideQuestionCancel, AgentChatSideQuestionCancelled, AgentChatSideQuestionOutcome,
    AgentChatSideQuestionRecord, AgentChatSideQuestionStatus, Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};

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

fn persist_ask(
    ledger: &SqliteLedger,
    ask: &AgentChatSideQuestion,
    side_question_id: &str,
) -> Result<AgentChatSideQuestionAsked, LedgerError> {
    validate_ask(ask, side_question_id)?;
    let idempotency_key = idempotency_key("gent-agent-chat-side-question-ask-v1", &ask.request_id.0);
    let command = ask_command(ask, side_question_id, &idempotency_key);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(ask.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
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

fn conversation_exists(transaction: &Transaction<'_>, conversation_id: &str) -> Result<bool, LedgerError> {
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

fn enforce_live_bounds(transaction: &Transaction<'_>, conversation_id: &str) -> Result<(), LedgerError> {
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

fn ask_command(ask: &AgentChatSideQuestion, side_question_id: &str, idempotency_key: &str) -> Command {
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

fn persist_completion(
    ledger: &SqliteLedger,
    side_question_id: &str,
    outcome: &AgentChatSideQuestionOutcome,
) -> Result<AgentChatSideQuestionRecord, LedgerError> {
    if side_question_id.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat side question identity must be nonempty".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let (status, answer, failure_reason) = match outcome {
        AgentChatSideQuestionOutcome::Answered { text } => ("answered", Some(text.clone()), None),
        AgentChatSideQuestionOutcome::Failed { reason } => ("failed", None, Some(reason.clone())),
    };
    transaction
        .execute(
            "UPDATE agent_chat_side_questions SET status = ?2, answer = ?3, failure_reason = ?4 WHERE side_question_id = ?1 AND status = 'pending'",
            params![side_question_id, status, answer, failure_reason],
        )
        .map_err(storage_error)?;
    let record = find(&transaction, side_question_id)?.ok_or_else(|| {
        LedgerError::Invariant("agent chat side question is unknown".into())
    })?;
    transaction.commit().map_err(storage_error)?;
    Ok(record)
}

fn persist_cancel(
    ledger: &SqliteLedger,
    cancel: &AgentChatSideQuestionCancel,
) -> Result<AgentChatSideQuestionCancelled, LedgerError> {
    if cancel.receipt_id.0.trim().is_empty()
        || cancel.request_id.0.trim().is_empty()
        || cancel.side_question_id.trim().is_empty()
    {
        return Err(LedgerError::Invariant(
            "agent chat side question cancel identities must be nonempty".into(),
        ));
    }
    let idempotency_key = idempotency_key(
        "gent-agent-chat-side-question-cancel-v1",
        &cancel.request_id.0,
    );
    let command = cancel_command(cancel, &idempotency_key);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(cancel.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(existing) = existing_cancel(&transaction, &idempotency_key, cancel)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(conflict("cancel"));
        }
        return Ok(existing);
    }
    if find_receipt(&transaction, &idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat side question cancel idempotency key is owned by another command".into(),
        ));
    }
    transaction
        .execute(
            "UPDATE agent_chat_side_questions SET status = 'cancelled' WHERE side_question_id = ?1 AND status = 'pending'",
            [&cancel.side_question_id],
        )
        .map_err(storage_error)?;
    let record = find(&transaction, &cancel.side_question_id)?.ok_or_else(|| {
        LedgerError::Invariant("agent chat side question is unknown".into())
    })?;
    let receipt = Receipt {
        receipt_id: cancel.receipt_id.clone(),
        idempotency_key: idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: cancel.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_side_question_cancel_receipts (idempotency_key, side_question_id) VALUES (?1, ?2)",
            params![idempotency_key, cancel.side_question_id],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatSideQuestionCancelled { receipt, record })
}

fn existing_cancel(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    cancel: &AgentChatSideQuestionCancel,
) -> Result<Option<AgentChatSideQuestionCancelled>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, c.side_question_id FROM agent_chat_side_question_cancel_receipts c JOIN receipts r ON r.idempotency_key = c.idempotency_key WHERE c.idempotency_key = ?1",
            [idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((receipt_id, status, epoch, side_question_id)) = row else {
        return Ok(None);
    };
    if receipt_id != cancel.receipt_id.0 || side_question_id != cancel.side_question_id || status != "settled"
    {
        return Err(conflict("cancel"));
    }
    let record = find(transaction, &side_question_id)?.ok_or_else(|| {
        LedgerError::Invariant("agent chat side question is unknown".into())
    })?;
    Ok(Some(AgentChatSideQuestionCancelled {
        receipt: Receipt {
            receipt_id: cancel.receipt_id.clone(),
            idempotency_key: idempotency_key.to_owned(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        record,
    }))
}

fn cancel_command(cancel: &AgentChatSideQuestionCancel, idempotency_key: &str) -> Command {
    Command {
        receipt_id: cancel.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        host_epoch: cancel.host_epoch,
        kind: "agentChatCancelSideQuestion".into(),
        payload: json!({ "sideQuestionId": cancel.side_question_id }),
    }
}

fn conflict(action: &str) -> LedgerError {
    LedgerError::Invariant(format!(
        "agent chat side question {action} retry conflicts with durable ownership"
    ))
}

fn find(
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

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<AgentChatSideQuestionRecord, LedgerError>> {
    let side_question_id: String = row.get(0)?;
    let conversation_id: String = row.get(1)?;
    let question: String = row.get(2)?;
    let status: String = row.get(3)?;
    let answer: Option<String> = row.get(4)?;
    let failure_reason: Option<String> = row.get(5)?;
    let created_at_unix_ms: u64 = row.get(6)?;
    Ok(parse_status(&status).map(|status| AgentChatSideQuestionRecord {
        side_question_id,
        conversation_id: AgentChatConversationId(conversation_id),
        question,
        status,
        answer,
        failure_reason,
        created_at_unix_ms,
    }))
}

fn parse_status(status: &str) -> Result<AgentChatSideQuestionStatus, LedgerError> {
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

fn idempotency_key(namespace: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("{:x}", digest.finalize())
}
