use super::{
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PromptProviderProvisionFrame,
    PromptProviderProvisionFrameError, PromptProviderProvisionState,
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
    assert!(
        serde_json::from_value::<PromptProviderProvisionFrame>(json!({
            "type": "confirm", "body": {
                "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
                "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
                "consentGranted": true, "reviewedPlanDigest": "a".repeat(64),
                "package": { "packageName": "@openai/codex" }
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
fn result_validation_fences_receipt_identity_epoch_and_terminal_state() {
    let valid = result();
    assert_eq!(valid.validate(), Ok(()));
    let mut invalid = valid.clone();
    receipt(&mut invalid).receipt_id = ReceiptId(" receipt".into());
    assert_eq!(
        invalid.validate(),
        Err(PromptProviderProvisionFrameError::InvalidIdentifier)
    );
    receipt(&mut invalid).receipt_id = ReceiptId("receipt".into());
    receipt(&mut invalid).host_epoch = HostEpoch(0);
    assert_eq!(
        invalid.validate(),
        Err(PromptProviderProvisionFrameError::InvalidValue)
    );
    receipt(&mut invalid).host_epoch = HostEpoch(1);
    receipt(&mut invalid).status = ReceiptStatus::Rejected;
    assert_eq!(
        invalid.validate(),
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
        state: PromptProviderProvisionState::Completed,
    }
}

fn receipt(frame: &mut PromptProviderProvisionFrame) -> &mut Receipt {
    let PromptProviderProvisionFrame::Result { receipt, .. } = frame else {
        unreachable!();
    };
    receipt
}
