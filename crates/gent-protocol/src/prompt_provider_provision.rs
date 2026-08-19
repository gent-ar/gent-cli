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
mod tests {
    use super::{
        PROMPT_PROVIDER_PROVISION_CAPABILITY, PromptProviderProvisionFrame,
        PromptProviderProvisionFrameError,
    };
    use gent_types::{
        AgentChatConversationId, AgentChatRunId, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
    };
    use serde_json::json;

    #[test]
    fn confirmation_cannot_inject_provider_native_or_plan_values() {
        assert_eq!(
            PROMPT_PROVIDER_PROVISION_CAPABILITY,
            "prompt-provider-provision-v1"
        );
        assert!(
            serde_json::from_value::<PromptProviderProvisionFrame>(json!({
                "type": "confirm", "body": {
                    "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
                    "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
                    "consentGranted": true, "reviewedPlanDigest": "a".repeat(64)
                }
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PromptProviderProvisionFrame>(json!({
                "type": "confirm", "body": {
                    "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
                    "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
                    "consentGranted": true, "reviewedPlanDigest": "a".repeat(64),
                    "provider": "codex"
                }
            }))
            .is_err()
        );
    }

    #[test]
    fn confirmation_validation_fences_identifiers_epoch_and_digest() {
        assert_eq!(confirm().validate(), Ok(()));
        let mut invalid_identifier = confirm();
        let PromptProviderProvisionFrame::Confirm {
            idempotency_key, ..
        } = &mut invalid_identifier
        else {
            unreachable!();
        };
        *idempotency_key = "\n".into();
        assert_eq!(
            invalid_identifier.validate(),
            Err(PromptProviderProvisionFrameError::InvalidIdentifier)
        );
        let mut invalid_epoch = confirm();
        let PromptProviderProvisionFrame::Confirm { host_epoch, .. } = &mut invalid_epoch else {
            unreachable!();
        };
        *host_epoch = HostEpoch(0);
        assert_eq!(
            invalid_epoch.validate(),
            Err(PromptProviderProvisionFrameError::InvalidValue)
        );
        let mut invalid_digest = confirm();
        let PromptProviderProvisionFrame::Confirm {
            reviewed_plan_digest,
            ..
        } = &mut invalid_digest
        else {
            unreachable!();
        };
        *reviewed_plan_digest = "A".repeat(64);
        assert_eq!(
            invalid_digest.validate(),
            Err(PromptProviderProvisionFrameError::InvalidValue)
        );
    }

    #[test]
    fn result_validation_fences_receipt_identity_and_epoch() {
        let valid = result();
        assert_eq!(valid.validate(), Ok(()));
        let mut invalid_receipt = valid.clone();
        {
            let PromptProviderProvisionFrame::Result { receipt, .. } = &mut invalid_receipt else {
                unreachable!();
            };
            receipt.receipt_id = ReceiptId(" receipt".into());
        }
        assert_eq!(
            invalid_receipt.validate(),
            Err(PromptProviderProvisionFrameError::InvalidIdentifier)
        );
        {
            let PromptProviderProvisionFrame::Result { receipt, .. } = &mut invalid_receipt else {
                unreachable!();
            };
            receipt.receipt_id = ReceiptId("receipt".into());
            receipt.host_epoch = HostEpoch(0);
        }
        assert_eq!(
            invalid_receipt.validate(),
            Err(PromptProviderProvisionFrameError::InvalidValue)
        );
    }

    fn confirm() -> PromptProviderProvisionFrame {
        PromptProviderProvisionFrame::Confirm {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "key".into(),
            host_epoch: HostEpoch(1),
            prompt_receipt_id: ReceiptId("prompt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
        }
    }

    fn result() -> PromptProviderProvisionFrame {
        PromptProviderProvisionFrame::Result {
            receipt: Receipt {
                receipt_id: ReceiptId("receipt".into()),
                idempotency_key: "key".into(),
                status: ReceiptStatus::Settled,
                host_epoch: HostEpoch(1),
            },
            prompt_receipt_id: ReceiptId("prompt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            state: super::PromptProviderProvisionState::Completed,
        }
    }
}
