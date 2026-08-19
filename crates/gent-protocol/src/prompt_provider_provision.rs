//! Capability-gated consent frames for installing a provider required by one held prompt.

use gent_types::{AgentChatConversationId, AgentChatRunId, HostEpoch, Receipt, ReceiptId};
use serde::{Deserialize, Serialize};

/// Required before a client may confirm a daemon-derived prompt-provider install review.
pub const PROMPT_PROVIDER_PROVISION_CAPABILITY: &str = "prompt-provider-provision-v1";
/// Maximum encoded size for one prompt-provider-provision exchange frame.
pub const MAX_PROMPT_PROVIDER_PROVISION_FRAME_BYTES: usize = 8 * 1024;

/// Daemon-owned result of a prompt-provider provision confirmation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptProviderProvisionState {
    Completed,
    ConsentRequired,
    Failed,
    PlanMismatch,
    Unprovable,
}

/// Request/reply frames for the one exact prompt a daemon readiness review placed on hold.
///
/// The request deliberately excludes provider, action, package, executable, plan, and policy.
/// Gent derives them from the durable selected run and its dependency catalog.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PromptProviderProvisionFrame {
    Confirm {
        receipt_id: ReceiptId,
        idempotency_key: String,
        host_epoch: HostEpoch,
        prompt_receipt_id: ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        consent_granted: bool,
        reviewed_plan_digest: String,
    },
    Result {
        receipt: Receipt,
        prompt_receipt_id: ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        state: PromptProviderProvisionState,
    },
}

impl PromptProviderProvisionFrame {
    /// Validates bounded correlation, epoch, and review-digest values before transport.
    ///
    /// # Errors
    /// Returns an error for malformed values or a frame outside the endpoint byte budget.
    pub fn validate(&self) -> Result<(), PromptProviderProvisionFrameError> {
        match self {
            Self::Confirm {
                receipt_id,
                idempotency_key,
                host_epoch,
                prompt_receipt_id,
                conversation_id,
                run_id,
                reviewed_plan_digest,
                ..
            } => {
                validate_ids(&[
                    &receipt_id.0,
                    idempotency_key,
                    &prompt_receipt_id.0,
                    &conversation_id.0,
                    &run_id.0,
                ])?;
                validate_epoch(*host_epoch)?;
                validate_digest(reviewed_plan_digest)?;
            }
            Self::Result {
                receipt,
                prompt_receipt_id,
                conversation_id,
                run_id,
                state,
                ..
            } => {
                validate_ids(&[
                    &receipt.receipt_id.0,
                    &receipt.idempotency_key,
                    &prompt_receipt_id.0,
                    &conversation_id.0,
                    &run_id.0,
                ])?;
                validate_epoch(receipt.host_epoch)?;
                valid_terminal_state(*state, receipt.status.clone())?;
            }
        }
        if self.encoded_len()? > MAX_PROMPT_PROVIDER_PROVISION_FRAME_BYTES {
            return Err(PromptProviderProvisionFrameError::TooLarge);
        }
        Ok(())
    }

    fn encoded_len(&self) -> Result<usize, PromptProviderProvisionFrameError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| PromptProviderProvisionFrameError::InvalidEncoding)
    }
}

fn valid_terminal_state(
    state: PromptProviderProvisionState,
    status: gent_types::ReceiptStatus,
) -> Result<(), PromptProviderProvisionFrameError> {
    matches!(
        (state, status),
        (
            PromptProviderProvisionState::Completed,
            gent_types::ReceiptStatus::Settled
        ) | (
            PromptProviderProvisionState::ConsentRequired
                | PromptProviderProvisionState::Failed
                | PromptProviderProvisionState::PlanMismatch,
            gent_types::ReceiptStatus::Rejected
        ) | (
            PromptProviderProvisionState::Unprovable,
            gent_types::ReceiptStatus::Unprovable
        )
    )
    .then_some(())
    .ok_or(PromptProviderProvisionFrameError::InvalidValue)
}

fn validate_ids(values: &[&str]) -> Result<(), PromptProviderProvisionFrameError> {
    values
        .iter()
        .all(|value| valid_id(value))
        .then_some(())
        .ok_or(PromptProviderProvisionFrameError::InvalidIdentifier)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 128
        && !value.chars().any(char::is_control)
}

fn validate_epoch(epoch: HostEpoch) -> Result<(), PromptProviderProvisionFrameError> {
    (epoch.0 != 0)
        .then_some(())
        .ok_or(PromptProviderProvisionFrameError::InvalidValue)
}

fn validate_digest(value: &str) -> Result<(), PromptProviderProvisionFrameError> {
    (value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(())
    .ok_or(PromptProviderProvisionFrameError::InvalidValue)
}

/// Value-free validation errors for prompt-provider-provision frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PromptProviderProvisionFrameError {
    #[error("prompt provider provision identifier is invalid")]
    InvalidIdentifier,
    #[error("prompt provider provision value is invalid")]
    InvalidValue,
    #[error("prompt provider provision frame exceeds byte budget")]
    TooLarge,
    #[error("prompt provider provision frame could not be encoded")]
    InvalidEncoding,
}

#[cfg(test)]
#[path = "prompt_provider_provision_tests.rs"]
mod tests;
