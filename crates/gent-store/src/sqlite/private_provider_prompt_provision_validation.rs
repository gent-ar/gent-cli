use gent_ports::LedgerError;
use gent_types::{
    Command, Event, ProviderPromptProvisionCommandBinding, ProvisionedProviderInstallation,
    Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, params};

use super::super::queries::storage_error;

pub(super) fn validate(
    command: &Command,
    receipt: &Receipt,
    installation: &ProvisionedProviderInstallation,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    checked(command, binding)?;
    (binding.prompt.consent_granted
        && command_matches_receipt(command, receipt)
        && terminal_matches(receipt, terminal, "privatePromptProvisionInstalled")
        && installation.lock.run_lock.provider == binding.prompt.provider
        && installation.provenance.package_name == binding.package.package_name
        && installation.provenance.package_version == binding.package.version
        && installation.provenance.package_integrity == binding.package.integrity
        && installation.provenance.package_policy_digest_sha256
            == binding.package.package_policy_digest_sha256)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("invalid prompt provision settlement".into()))
}

pub(super) fn validate_admission(
    command: &Command,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    checked(command, binding)?;
    binding
        .prompt
        .consent_granted
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("invalid prompt provision admission".into()))
}

pub(super) fn validate_unprovable(
    command: &Command,
    receipt: &Receipt,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    validate_admission(command, binding)?;
    (command_matches_receipt(command, receipt)
        && terminal_matches(receipt, terminal, "privatePromptProvisionUnprovable"))
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("invalid prompt provision failure settlement".into()))
}

pub(super) fn validate_pre_effect_failure(
    command: &Command,
    receipt: &Receipt,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    validate_admission(command, binding)?;
    (command_receipt_identity(command, receipt)
        && matches!(
            receipt.status,
            ReceiptStatus::Accepted | ReceiptStatus::Rejected
        )
        && terminal_matches(receipt, terminal, "privatePromptProvisionFailed")
        && terminal.payload == command.payload)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("invalid prompt provision pre-effect failure".into()))
}

pub(super) fn validate_rejection_admission(
    command: &Command,
    terminal: &Event,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    checked(command, binding)?;
    (terminal.receipt_id == command.receipt_id
        && terminal.host_epoch == command.host_epoch
        && terminal.payload == command.payload
        && rejection_matches(binding, terminal))
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("invalid prompt provision rejection settlement".into()))
}

fn rejection_matches(binding: &ProviderPromptProvisionCommandBinding, terminal: &Event) -> bool {
    match terminal.kind.as_str() {
        "privatePromptProvisionConsentRequired" => !binding.prompt.consent_granted,
        "privatePromptProvisionPlanMismatch" => {
            binding.prompt.consent_granted
                && binding.prompt.reviewed_plan_digest != binding.expected_reviewed_plan_digest
        }
        _ => false,
    }
}

pub(super) fn prompt_message(
    transaction: &rusqlite::Transaction<'_>,
    binding: &ProviderPromptProvisionCommandBinding,
    state: &str,
) -> Result<String, LedgerError> {
    let message_id = transaction
        .query_row(
            "SELECT p.message_id FROM agent_chat_prompt_receipts p JOIN receipts prompt_receipt ON prompt_receipt.idempotency_key = p.idempotency_key JOIN agent_chat_prompt_dispatches d ON d.message_id = p.message_id JOIN agent_chat_run_selections selected ON selected.run_id = p.run_id WHERE prompt_receipt.receipt_id = ?1 AND p.conversation_id = ?2 AND p.run_id = ?3 AND p.disposition = 'send' AND selected.provider = ?4 AND d.state = ?5 AND p.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections current_selected ON current_selected.run_id = current.run_id WHERE current.conversation_id = p.conversation_id ORDER BY current.rowid DESC LIMIT 1)",
            params![binding.prompt.prompt_receipt_id.0, binding.prompt.conversation_id.0, binding.prompt.run_id.0, binding.prompt.provider, state],
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

fn checked(
    command: &Command,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), LedgerError> {
    let payload = serde_json::to_value(binding).map_err(storage_error)?;
    (binding.is_valid()
        && command.kind == "providerPromptProvision"
        && command.payload == payload
        && !command.receipt_id.0.trim().is_empty()
        && !command.idempotency_key.trim().is_empty())
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("invalid prompt provision command".into()))
}

fn command_matches_receipt(command: &Command, receipt: &Receipt) -> bool {
    command_receipt_identity(command, receipt) && receipt.status == ReceiptStatus::Accepted
}

fn command_receipt_identity(command: &Command, receipt: &Receipt) -> bool {
    command.receipt_id == receipt.receipt_id
        && command.idempotency_key == receipt.idempotency_key
        && command.host_epoch == receipt.host_epoch
}

fn terminal_matches(receipt: &Receipt, terminal: &Event, kind: &str) -> bool {
    terminal.receipt_id == receipt.receipt_id
        && terminal.host_epoch == receipt.host_epoch
        && terminal.kind == kind
}
