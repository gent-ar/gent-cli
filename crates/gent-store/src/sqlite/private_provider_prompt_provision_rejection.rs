use gent_ports::{IngressMode, LedgerError};
use gent_types::{Command, Event, ProviderPromptProvisionCommandBinding, Receipt, ReceiptStatus};
use rusqlite::TransactionBehavior;

use super::super::{
    epoch::require_epoch,
    queries::{
        append_event, find_event, find_receipt, host_ingress, insert_receipt,
        receipt_matches_command, storage_error,
    },
};
use super::{
    SqliteLedger,
    validation::{prompt_message, validate_rejection_admission},
};

pub(super) fn settle(
    ledger: &SqliteLedger,
    command: &Command,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<Receipt, LedgerError> {
    validate_rejection_admission(command, terminal, binding)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(command.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
        let exact_terminal = find_event(&transaction, &terminal.event_id)?.is_some_and(|event| {
            event.receipt_id == terminal.receipt_id
                && event.host_epoch == terminal.host_epoch
                && event.kind == terminal.kind
                && event.payload == terminal.payload
        });
        return (receipt.status == ReceiptStatus::Rejected
            && receipt_matches_command(&transaction, command)?
            && exact_terminal)
            .then_some(receipt)
            .ok_or_else(|| LedgerError::Invariant("prompt provision already admitted".into()));
    }
    prompt_message(&transaction, binding, "awaiting_readiness")?;
    let receipt = Receipt {
        receipt_id: command.receipt_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        status: ReceiptStatus::Rejected,
        host_epoch: ingress.epoch,
    };
    insert_receipt(&transaction, &receipt, command)?;
    append_event(&transaction, terminal)?;
    transaction.commit().map_err(storage_error)?;
    Ok(receipt)
}
