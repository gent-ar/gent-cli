use gent_ports::LedgerError;
use gent_types::{
    AgentChatSideQuestionCancel, AgentChatSideQuestionCancelled, AgentChatSideQuestionOutcome,
    AgentChatSideQuestionRecord, Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::super::SqliteLedger;
use super::super::super::queries::{
    find_receipt, insert_receipt, receipt_matches_command, storage_error,
};
use super::super::prompt_dispatch::require_open;
use super::records::find;
use super::{conflict, idempotency_key};

pub(super) fn persist_completion(
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
    let record = find(&transaction, side_question_id)?
        .ok_or_else(|| LedgerError::Invariant("agent chat side question is unknown".into()))?;
    transaction.commit().map_err(storage_error)?;
    Ok(record)
}

pub(super) fn persist_cancel(
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
    require_open(&transaction, cancel.host_epoch)?;
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
    let record = find(&transaction, &cancel.side_question_id)?
        .ok_or_else(|| LedgerError::Invariant("agent chat side question is unknown".into()))?;
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
    if receipt_id != cancel.receipt_id.0
        || side_question_id != cancel.side_question_id
        || status != "settled"
    {
        return Err(conflict("cancel"));
    }
    let record = find(transaction, &side_question_id)?
        .ok_or_else(|| LedgerError::Invariant("agent chat side question is unknown".into()))?;
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
