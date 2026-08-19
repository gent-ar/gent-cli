use gent_ports::LedgerError;
use gent_types::{Command, Event, ProviderPromptProvisionCommandBinding, Receipt, ReceiptStatus};
use rusqlite::{TransactionBehavior, params};

use super::super::queries::{
    append_event, encode_status, find_event, find_receipt, receipt_matches_command, storage_error,
};
use super::{
    SqliteLedger,
    validation::{prompt_message, validate_pre_effect_failure},
};

pub(super) fn settle(
    ledger: &SqliteLedger,
    command: &Command,
    receipt: &Receipt,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<Receipt, LedgerError> {
    validate_pre_effect_failure(command, receipt, terminal, binding)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let durable = find_receipt(&transaction, &receipt.idempotency_key)?
        .ok_or_else(|| LedgerError::Invariant("prompt provision receipt is missing".into()))?;
    if durable.receipt_id != receipt.receipt_id
        || durable.host_epoch != receipt.host_epoch
        || !receipt_matches_command(&transaction, command)?
    {
        return Err(LedgerError::Invariant(
            "prompt provision receipt no longer matches its command".into(),
        ));
    }
    if durable.status == ReceiptStatus::Rejected {
        return exact_terminal(&transaction, terminal)?
            .then_some(durable)
            .ok_or_else(|| {
                LedgerError::Invariant("prompt provision terminal event differs".into())
            });
    }
    if durable.status != ReceiptStatus::Accepted {
        return Err(LedgerError::Invariant(
            "prompt provision already terminally settled".into(),
        ));
    }
    let message_id = prompt_message(&transaction, binding, "provisioning")?;
    transaction
        .execute(
            "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
            params![
                encode_status(&ReceiptStatus::Rejected),
                receipt.idempotency_key
            ],
        )
        .map_err(storage_error)?;
    append_event(&transaction, terminal)?;
    let released = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'awaiting_readiness' WHERE message_id = ?1 AND state = 'provisioning'",
            [message_id],
        )
        .map_err(storage_error)?;
    if released != 1 {
        return Err(LedgerError::Invariant(
            "prompt provision reservation changed before pre-effect rejection".into(),
        ));
    }
    transaction.commit().map_err(storage_error)?;
    Ok(Receipt {
        status: ReceiptStatus::Rejected,
        ..durable
    })
}

fn exact_terminal(
    transaction: &rusqlite::Transaction<'_>,
    terminal: &Event,
) -> Result<bool, LedgerError> {
    Ok(
        find_event(transaction, &terminal.event_id)?.is_some_and(|event| {
            event.receipt_id == terminal.receipt_id
                && event.host_epoch == terminal.host_epoch
                && event.kind == terminal.kind
                && event.payload == terminal.payload
        }),
    )
}
