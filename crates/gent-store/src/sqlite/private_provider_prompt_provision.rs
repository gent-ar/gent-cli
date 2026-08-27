//! One `SQLite` transaction for verified private provisioning and its held-prompt release.

use gent_ports::{IngressMode, LedgerError, PrivateProviderPromptProvisionLedger, ReceiptClaim};
use gent_types::{
    Command, Event, ProviderPromptProvisionCommandBinding, ProvisionedProviderInstallation,
    Receipt, ReceiptStatus,
};
use rusqlite::{TransactionBehavior, params};

use super::{
    SqliteLedger,
    epoch::require_epoch,
    provisioned_provider_locks::{existing_settlement, save_installation},
    queries::{
        append_event, encode_status, find_receipt, host_ingress, insert_receipt,
        receipt_matches_command, storage_error,
    },
};

#[path = "private_provider_prompt_provision_pre_effect_failure.rs"]
mod pre_effect_failure;
#[path = "private_provider_prompt_provision_rejection.rs"]
mod rejection;
#[path = "private_provider_prompt_provision_unprovable.rs"]
mod unprovable;
#[path = "private_provider_prompt_provision_validation.rs"]
mod validation;

use validation::{prompt_message, validate, validate_admission};

impl PrivateProviderPromptProvisionLedger for SqliteLedger {
    fn claim_and_reserve_verified_provider_prompt_provision(
        &self,
        command: &Command,
        accepted: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<ReceiptClaim, LedgerError> {
        validate_admission(command, binding)?;
        let mut connection = self.lock()?;
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
        if accepted.receipt_id != command.receipt_id
            || accepted.host_epoch != command.host_epoch
            || accepted.kind != "privatePromptProvisionAccepted"
            || accepted.payload != command.payload
        {
            return Err(LedgerError::Invariant(
                "invalid prompt provision accepted event".into(),
            ));
        }
        if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
            if !receipt_matches_command(&transaction, command)? {
                return Err(LedgerError::Invariant(
                    "idempotency key is bound to a different command".into(),
                ));
            }
            if receipt.status == ReceiptStatus::Accepted {
                prompt_message(&transaction, binding, "provisioning")?;
            }
            return Ok(ReceiptClaim::Existing(receipt));
        }
        let receipt = Receipt {
            receipt_id: command.receipt_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            status: ReceiptStatus::Accepted,
            host_epoch: ingress.epoch,
        };
        insert_receipt(&transaction, &receipt, command)?;
        append_event(&transaction, accepted)?;
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
        transaction.commit().map_err(storage_error)?;
        Ok(ReceiptClaim::Accepted(receipt))
    }

    fn reserve_verified_provider_prompt_provision(
        &self,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<(), LedgerError> {
        reserve(self, command, binding)
    }

    fn settle_verified_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        installation: &ProvisionedProviderInstallation,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
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

    fn settle_unprovable_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError> {
        unprovable::settle(self, command, receipt, terminal, binding)
    }

    fn reject_verified_provider_prompt_provision(
        &self,
        command: &Command,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError> {
        rejection::settle(self, command, terminal, binding)
    }

    fn reject_pre_effect_verified_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError> {
        pre_effect_failure::settle(self, command, receipt, terminal, binding)
    }
}

fn reserve(
    ledger: &SqliteLedger,
    command: &Command,
    binding: &ProviderPromptProvisionCommandBinding,
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

#[cfg(test)]
#[path = "private_provider_prompt_provision_failure_tests.rs"]
mod failure_tests;
#[cfg(test)]
#[path = "private_provider_prompt_provision_package_tests.rs"]
mod package_tests;
#[cfg(test)]
#[path = "private_provider_prompt_provision_release_tests.rs"]
mod release_tests;
#[cfg(test)]
#[path = "private_provider_prompt_provision_reservation_tests.rs"]
mod reservation_tests;
#[cfg(test)]
#[path = "private_provider_prompt_provision_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "private_provider_prompt_provision_tests.rs"]
mod tests;
