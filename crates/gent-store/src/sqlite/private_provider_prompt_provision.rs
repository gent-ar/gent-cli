//! One `SQLite` transaction for verified private provisioning and its held-prompt release.

use gent_ports::{IngressMode, LedgerError, PrivateProviderPromptProvisionLedger};
use gent_types::{
    Command, Event, ProviderPromptProvisionBinding, ProvisionedProviderInstallation, Receipt,
    ReceiptStatus,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{
    SqliteLedger,
    epoch::require_epoch,
    provisioned_provider_locks::{existing_settlement, save_installation},
    queries::{
        append_event, encode_status, find_receipt, host_ingress, receipt_matches_command,
        storage_error,
    },
};

impl PrivateProviderPromptProvisionLedger for SqliteLedger {
    fn reserve_verified_provider_prompt_provision(
        &self,
        command: &Command,
        binding: &ProviderPromptProvisionBinding,
    ) -> Result<(), LedgerError> {
        reserve(self, command, binding)
    }

    fn settle_verified_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        installation: &ProvisionedProviderInstallation,
        terminal: &Event,
        binding: &ProviderPromptProvisionBinding,
    ) -> Result<Receipt, LedgerError> {
        validate(command, receipt, installation, terminal, binding)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let ingress = host_ingress(&transaction)?;
        require_epoch(receipt.host_epoch, ingress.epoch)?;
        if ingress.mode == IngressMode::Closed {
            return Err(LedgerError::IngressClosed {
                epoch: ingress.epoch,
            });
        }
        let durable = find_receipt(&transaction, &receipt.idempotency_key)?
            .ok_or_else(|| LedgerError::Invariant("prompt provision receipt is missing".into()))?;
        if durable.receipt_id != receipt.receipt_id
            || durable.idempotency_key != receipt.idempotency_key
            || durable.host_epoch != receipt.host_epoch
            || !receipt_matches_command(&transaction, command)?
        {
            return Err(LedgerError::Invariant(
                "prompt provision receipt no longer matches its command".into(),
            ));
        }
        if durable.status != ReceiptStatus::Accepted {
            return existing_settlement(
                &transaction,
                &durable,
                Some(installation),
                &ReceiptStatus::Settled,
            );
        }
        let message_id = prompt_message(&transaction, binding, "provisioning")?;
        save_installation(&transaction, receipt, installation)?;
        transaction
            .execute(
                "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
                params![
                    encode_status(&ReceiptStatus::Settled),
                    receipt.idempotency_key
                ],
            )
            .map_err(storage_error)?;
        append_event(&transaction, terminal)?;
        let released = transaction
            .execute(
                "UPDATE agent_chat_prompt_dispatches SET state = 'pending' WHERE message_id = ?1 AND state = 'provisioning'",
                [message_id],
            )
            .map_err(storage_error)?;
        if released != 1 {
            return Err(LedgerError::Invariant(
                "prompt provision held dispatch changed before settlement".into(),
            ));
        }
        transaction.commit().map_err(storage_error)?;
        Ok(Receipt {
            status: ReceiptStatus::Settled,
            ..durable
        })
    }
}

fn validate(
    command: &Command,
    receipt: &Receipt,
    installation: &ProvisionedProviderInstallation,
    terminal: &Event,
    binding: &ProviderPromptProvisionBinding,
) -> Result<(), LedgerError> {
    let payload = serde_json::to_value(binding).map_err(storage_error)?;
    (binding.is_valid()
        && binding.consent_granted
        && command.kind == "providerPromptProvision"
        && command.payload == payload
        && command.receipt_id == receipt.receipt_id
        && command.idempotency_key == receipt.idempotency_key
        && command.host_epoch == receipt.host_epoch
        && receipt.status == ReceiptStatus::Accepted
        && terminal.receipt_id == receipt.receipt_id
        && terminal.host_epoch == receipt.host_epoch
        && terminal.kind == "privatePromptProvisionInstalled"
        && installation.lock.run_lock.provider == binding.provider)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("invalid prompt provision settlement".into()))
}

fn reserve(
    ledger: &SqliteLedger,
    command: &Command,
    binding: &ProviderPromptProvisionBinding,
) -> Result<(), LedgerError> {
    validate_admission(command, binding)?;
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
    let receipt = find_receipt(&transaction, &command.idempotency_key)?
        .ok_or_else(|| LedgerError::Invariant("prompt provision receipt is missing".into()))?;
    if receipt.status != ReceiptStatus::Accepted || !receipt_matches_command(&transaction, command)?
    {
        return Err(LedgerError::Invariant(
            "prompt provision receipt no longer matches its accepted command".into(),
        ));
    }
    let message_id = prompt_message(&transaction, binding, "awaiting_readiness")?;
    let reserved = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'provisioning' WHERE message_id = ?1 AND state = 'awaiting_readiness'",
            [message_id],
        )
        .map_err(storage_error)?;
    if reserved != 1 {
        return Err(LedgerError::Invariant(
            "prompt provision held dispatch changed before admission".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}

fn validate_admission(
    command: &Command,
    binding: &ProviderPromptProvisionBinding,
) -> Result<(), LedgerError> {
    let payload = serde_json::to_value(binding).map_err(storage_error)?;
    (binding.is_valid()
        && binding.consent_granted
        && command.kind == "providerPromptProvision"
        && command.payload == payload
        && !command.receipt_id.0.trim().is_empty()
        && !command.idempotency_key.trim().is_empty())
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("invalid prompt provision admission".into()))
}

fn prompt_message(
    transaction: &rusqlite::Transaction<'_>,
    binding: &ProviderPromptProvisionBinding,
    state: &str,
) -> Result<String, LedgerError> {
    let message_id = transaction
        .query_row(
            "SELECT p.message_id FROM agent_chat_prompt_receipts p JOIN receipts prompt_receipt ON prompt_receipt.idempotency_key = p.idempotency_key JOIN agent_chat_prompt_dispatches d ON d.message_id = p.message_id JOIN agent_chat_run_selections selected ON selected.run_id = p.run_id WHERE prompt_receipt.receipt_id = ?1 AND p.conversation_id = ?2 AND p.run_id = ?3 AND p.disposition = 'send' AND selected.provider = ?4 AND d.state = ?5 AND p.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections current_selected ON current_selected.run_id = current.run_id WHERE current.conversation_id = p.conversation_id ORDER BY current.rowid DESC LIMIT 1)",
            params![
                binding.prompt_receipt_id.0,
                binding.conversation_id.0,
                binding.run_id.0,
                binding.provider,
                state,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?;
    message_id.ok_or_else(|| {
        LedgerError::Invariant(
            "prompt provision does not own a held prompt for the current provider run".into(),
        )
    })
}

#[cfg(test)]
#[path = "private_provider_prompt_provision_tests.rs"]
mod tests;
