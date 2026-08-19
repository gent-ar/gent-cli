//! Atomic release of a prompt held until private provider readiness is proven.

use gent_ports::LedgerError;
use gent_types::{
    AgentChatProvider, AgentChatRunId, Command, Event, HostEpoch, ProviderPromptReadinessBinding,
    Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::SqliteLedger;
use super::super::queries::{
    append_event, find_event, find_receipt, insert_receipt, receipt_matches_command, storage_error,
};
use super::prompt_dispatch::require_open;

/// Promotes only the exact current selected run from held to claimable dispatch.
pub(super) fn release(
    ledger: &SqliteLedger,
    message_id: &str,
    expected_run_id: &AgentChatRunId,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    if message_id.trim().is_empty() || expected_run_id.0.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat readiness release identity is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'pending' WHERE message_id = ?1 AND state = 'awaiting_readiness' AND EXISTS (SELECT 1 FROM conversation_messages m JOIN agent_chat_run_selections selected ON selected.run_id = m.run_id WHERE m.message_id = ?1 AND m.run_id = ?2 AND m.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections current_selected ON current_selected.run_id = current.run_id WHERE current.conversation_id = m.conversation_id ORDER BY current.rowid DESC LIMIT 1))",
        params![message_id, expected_run_id.0],
    ).map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat prompt is not held for the current reviewed run".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}

/// Records one daemon-owned readiness proof before making only its held prompt claimable.
pub(super) fn release_verified(
    ledger: &SqliteLedger,
    command: &Command,
    terminal: &Event,
    binding: &ProviderPromptReadinessBinding,
) -> Result<Receipt, LedgerError> {
    validate(command, terminal, binding)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, command.host_epoch)?;
    if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
        return existing(&transaction, command, terminal, receipt);
    }
    receipt_id_is_free(&transaction, command)?;
    let message_id = held_message(&transaction, binding)?;
    let receipt = Receipt {
        receipt_id: command.receipt_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: command.host_epoch,
    };
    insert_receipt(&transaction, &receipt, command)?;
    append_event(&transaction, terminal)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'pending' WHERE message_id = ?1 AND state = 'awaiting_readiness'",
            [message_id],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "verified provider readiness changed before prompt release".into(),
        ));
    }
    transaction.commit().map_err(storage_error)?;
    Ok(receipt)
}

fn validate(
    command: &Command,
    terminal: &Event,
    binding: &ProviderPromptReadinessBinding,
) -> Result<(), LedgerError> {
    let payload = serde_json::to_value(binding).map_err(storage_error)?;
    (binding.is_valid()
        && command.kind == "agentChatProviderReadiness"
        && !command.receipt_id.0.trim().is_empty()
        && !command.idempotency_key.trim().is_empty()
        && command.payload == payload
        && terminal.receipt_id == command.receipt_id
        && terminal.host_epoch == command.host_epoch
        && terminal.kind == "agentChatProviderReady"
        && terminal.payload == payload
        && !terminal.event_id.trim().is_empty())
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("invalid verified provider readiness release".into()))
}

fn existing(
    transaction: &Transaction<'_>,
    command: &Command,
    terminal: &Event,
    receipt: Receipt,
) -> Result<Receipt, LedgerError> {
    let exact_event = find_event(transaction, &terminal.event_id)?.is_some_and(|event| {
        event.receipt_id == terminal.receipt_id
            && event.host_epoch == terminal.host_epoch
            && event.kind == terminal.kind
            && event.payload == terminal.payload
    });
    (receipt.status == ReceiptStatus::Settled
        && receipt_matches_command(transaction, command)?
        && exact_event)
        .then_some(receipt)
        .ok_or_else(|| LedgerError::Invariant("provider readiness receipt is already bound".into()))
}

fn receipt_id_is_free(transaction: &Transaction<'_>, command: &Command) -> Result<(), LedgerError> {
    let owner = transaction
        .query_row(
            "SELECT idempotency_key FROM receipts WHERE receipt_id = ?1",
            [&command.receipt_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    owner
        .is_none_or(|key| key == command.idempotency_key)
        .then_some(())
        .ok_or_else(|| {
            LedgerError::Invariant("provider readiness receipt id is already bound".into())
        })
}

fn held_message(
    transaction: &Transaction<'_>,
    binding: &ProviderPromptReadinessBinding,
) -> Result<String, LedgerError> {
    transaction
        .query_row(
            "SELECT p.message_id FROM agent_chat_prompt_receipts p JOIN receipts prompt_receipt ON prompt_receipt.idempotency_key = p.idempotency_key JOIN agent_chat_prompt_dispatches d ON d.message_id = p.message_id JOIN agent_chat_run_selections selected ON selected.run_id = p.run_id WHERE prompt_receipt.receipt_id = ?1 AND prompt_receipt.status = 'settled' AND p.conversation_id = ?2 AND p.run_id = ?3 AND p.disposition = 'send' AND selected.provider = ?4 AND d.state = 'awaiting_readiness' AND p.run_id = (SELECT current.run_id FROM runs current WHERE current.conversation_id = p.conversation_id ORDER BY current.rowid DESC LIMIT 1)",
            params![binding.prompt_receipt_id.0, binding.conversation_id.0, binding.run_id.0, provider_name(binding.provider)],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| LedgerError::Invariant("verified readiness does not own the current held prompt".into()))
}

const fn provider_name(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}
