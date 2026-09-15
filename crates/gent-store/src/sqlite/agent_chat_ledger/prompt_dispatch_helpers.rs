use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, HostEpoch,
    Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::{Transaction, TransactionBehavior, params};

use super::super::super::SqliteLedger;
use super::super::super::queries::storage_error;
use super::require_open;

pub(super) fn valid_owner(coordinator_id: &str) -> Result<(), LedgerError> {
    (!coordinator_id.trim().is_empty() && coordinator_id.len() <= 512)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("agent chat dispatch coordinator is invalid".into()))
}

pub(super) fn fail_prelaunch(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: gent_types::HostEpoch,
    error: &str,
) -> Result<(), LedgerError> {
    valid_owner(coordinator_id)?;
    if message_id.trim().is_empty() || error.is_empty() || error.len() > 64 * 1024 {
        return Err(LedgerError::Invariant(
            "agent chat prelaunch failure is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let (conversation_id, run_id, turn_id): (String, String, String) = transaction
        .query_row(
            "SELECT conversation_id, run_id, turn_id FROM conversation_messages WHERE message_id = ?1",
            [message_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(storage_error)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled' WHERE message_id = ?1 AND state = 'claimed' AND coordinator_id = ?2 AND host_epoch = ?3",
            params![message_id, coordinator_id, host_epoch.0],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    if !crate::sqlite::turn_terminal::settle(
        &transaction,
        &turn_id,
        host_epoch,
        gent_types::DurableTurnPhase::Failed,
    )? {
        return Err(LedgerError::Invariant(
            "agent chat terminal turn is not active".into(),
        ));
    }
    let cursor: u64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1",
            [&conversation_id],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES (?1, ?2, ?3, ?4, ?5, 'notice', ?6, 0)",
            params![conversation_id, cursor, format!("prelaunch-failed:{message_id}"), turn_id, run_id, error],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

pub(super) fn saved(
    transaction: &Transaction<'_>,
    message_id: &str,
) -> Result<AgentChatPromptSaved, LedgerError> {
    transaction
        .query_row(
            "SELECT r.receipt_id, r.idempotency_key, r.host_epoch, p.run_id, p.tool_source_ids_json, m.message_id, m.turn_id, m.conversation_id, t.sequence, m.text, m.text_digest_sha256, p.disposition FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN conversation_messages m ON m.message_id = p.message_id JOIN turns t ON t.turn_id = p.turn_id WHERE p.message_id = ?1",
            [message_id],
            |row| {
                Ok(AgentChatPromptSaved {
                    receipt: Receipt {
                        receipt_id: ReceiptId(row.get(0)?),
                        idempotency_key: row.get(1)?,
                        status: ReceiptStatus::Settled,
                        host_epoch: gent_types::HostEpoch(row.get(2)?),
                    },
                    run_id: AgentChatRunId(row.get(3)?),
                    tool_source_ids: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    message: gent_types::ConversationMessage {
                        message_id: row.get(5)?,
                        turn_id: row.get(6)?,
                        conversation_id: row.get(7)?,
                        run_id: row.get(3)?,
                        sequence: row.get(8)?,
                        text: row.get(9)?,
                        text_digest_sha256: row.get(10)?,
                    },
                    disposition: match row.get::<_, String>(11)?.as_str() {
                        "send" => AgentChatPromptDisposition::Send,
                        "queue" => AgentChatPromptDisposition::Queue,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    },
                    delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider,
                })
            },
        )
        .map_err(storage_error)
}

pub(super) fn has_pending(
    ledger: &SqliteLedger,
    provider: AgentChatProvider,
) -> Result<bool, LedgerError> {
    let connection = ledger.lock()?;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN agent_chat_run_selections s ON s.run_id = m.run_id WHERE s.provider = ?1 AND d.state = 'pending' AND m.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections selected ON selected.run_id = current.run_id WHERE current.conversation_id = m.conversation_id ORDER BY current.rowid DESC LIMIT 1))",
            params![provider_name(provider)],
            |row| row.get::<_, bool>(0),
        )
        .map_err(storage_error)
}

pub(super) const fn provider_name(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}

pub(super) fn transition(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    expected: &str,
    state: &str,
    retain_owner: bool,
) -> Result<(), LedgerError> {
    valid_owner(coordinator_id)?;
    if message_id.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat dispatch message is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = if retain_owner {
        transaction.execute(
            "UPDATE agent_chat_prompt_dispatches SET state = ?1 WHERE message_id = ?2 AND state = ?3 AND coordinator_id = ?4 AND host_epoch = ?5",
            params![state, message_id, expected, coordinator_id, host_epoch.0],
        )
    } else {
        transaction.execute(
            "UPDATE agent_chat_prompt_dispatches SET state = ?1, coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?2 AND state = ?3 AND coordinator_id = ?4 AND host_epoch = ?5",
            params![state, message_id, expected, coordinator_id, host_epoch.0],
        )
    }.map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}
