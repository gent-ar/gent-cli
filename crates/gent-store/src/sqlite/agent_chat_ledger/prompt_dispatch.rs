//! `SQLite` outbox ownership for durable provider-bound agent-chat prompts.

use gent_ports::{AgentChatPromptDispatchLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, HostEpoch,
    Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{host_ingress, storage_error};

impl AgentChatPromptDispatchLedger for SqliteLedger {
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        claim(self, coordinator_id, host_epoch, provider)
    }

    fn release_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "pending",
            false,
        )
    }

    fn settle_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "settled",
            true,
        )
    }
}

fn claim(
    ledger: &SqliteLedger,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    provider: AgentChatProvider,
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let message_id = transaction
        .query_row(
            "SELECT d.message_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN agent_chat_run_selections s ON s.run_id = m.run_id WHERE s.provider = ?1 AND (d.state = 'pending' OR (d.state = 'claimed' AND d.host_epoch < ?2)) ORDER BY d.created_rowid LIMIT 1",
            params![provider_name(provider), host_epoch.0],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    let Some(message_id) = message_id else {
        return Ok(None);
    };
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'claimed', coordinator_id = ?1, host_epoch = ?2 WHERE message_id = ?3",
        params![coordinator_id, host_epoch.0, message_id],
    ).map_err(storage_error)?;
    let saved = saved(&transaction, &message_id)?;
    transaction.commit().map_err(storage_error)?;
    Ok(Some(saved))
}

const fn provider_name(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}

fn transition(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
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
            "UPDATE agent_chat_prompt_dispatches SET state = ?1 WHERE message_id = ?2 AND state = 'claimed' AND coordinator_id = ?3 AND host_epoch = ?4",
            params![state, message_id, coordinator_id, host_epoch.0],
        )
    } else {
        transaction.execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'pending', coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?1 AND state = 'claimed' AND coordinator_id = ?2 AND host_epoch = ?3",
            params![message_id, coordinator_id, host_epoch.0],
        )
    }.map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}

fn require_open(transaction: &Transaction<'_>, host_epoch: HostEpoch) -> Result<(), LedgerError> {
    let ingress = host_ingress(transaction)?;
    require_epoch(host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    Ok(())
}

fn valid_owner(coordinator_id: &str) -> Result<(), LedgerError> {
    (!coordinator_id.trim().is_empty() && coordinator_id.len() <= 512)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("agent chat dispatch coordinator is invalid".into()))
}

fn saved(
    transaction: &Transaction<'_>,
    message_id: &str,
) -> Result<AgentChatPromptSaved, LedgerError> {
    transaction.query_row(
        "SELECT r.receipt_id, r.idempotency_key, r.host_epoch, p.run_id, m.message_id, m.turn_id, m.conversation_id, t.sequence, m.text, m.text_digest_sha256 FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN conversation_messages m ON m.message_id = p.message_id JOIN turns t ON t.turn_id = p.turn_id WHERE p.message_id = ?1",
        [message_id],
        |row| Ok(AgentChatPromptSaved {
            receipt: Receipt { receipt_id: ReceiptId(row.get(0)?), idempotency_key: row.get(1)?, status: ReceiptStatus::Settled, host_epoch: HostEpoch(row.get(2)?) },
            run_id: AgentChatRunId(row.get(3)?),
            message: gent_types::ConversationMessage { message_id: row.get(4)?, turn_id: row.get(5)?, conversation_id: row.get(6)?, run_id: row.get(3)?, sequence: row.get(7)?, text: row.get(8)?, text_digest_sha256: row.get(9)? },
            disposition: AgentChatPromptDisposition::Send,
            delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider,
        }),
    ).map_err(storage_error)
}
