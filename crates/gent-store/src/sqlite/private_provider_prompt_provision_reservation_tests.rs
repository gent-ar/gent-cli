use gent_ports::{Ledger, PrivateProviderPromptProvisionLedger, ReceiptClaim};
use gent_types::{
    AgentChatConversationId, Command, Event, HostEpoch, ProviderPromptProvisionBinding,
    ProviderPromptProvisionCommandBinding, ProviderPromptProvisionPackageBinding, ReceiptId,
    ReceiptStatus,
};
use serde_json::json;

use super::tests;

#[test]
fn atomic_claim_always_enters_provisioning_before_returning_accepted() {
    let (ledger, saved) = tests::seeded();
    let binding = binding(&saved, true, "a");
    let command = command(&binding, "reservation");
    let accepted = accepted(&command);
    assert!(matches!(
        ledger
            .claim_and_reserve_verified_provider_prompt_provision(&command, &accepted, &binding)
            .unwrap(),
        ReceiptClaim::Accepted(receipt) if receipt.status == ReceiptStatus::Accepted
    ));
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    assert!(matches!(
        ledger
            .claim_and_reserve_verified_provider_prompt_provision(&command, &accepted, &binding)
            .unwrap(),
        ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted
    ));
}

#[test]
fn rejected_consent_leaves_the_held_prompt_retryable() {
    let (ledger, saved) = tests::seeded();
    let binding = binding(&saved, false, "a");
    let command = command(&binding, "refusal");
    let receipt = match ledger.claim_command(&command, &accepted(&command)).unwrap() {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("fixture receipt must be new"),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "refusal:consent".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionConsentRequired".into(),
        payload: json!({}),
    };
    assert_eq!(
        ledger
            .settle_rejected_provider_prompt_provision(&command, &receipt, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Rejected
    );
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "awaiting_readiness"
    );
}

#[test]
fn stale_review_digest_is_rejected_without_reserving_the_prompt() {
    let (ledger, saved) = tests::seeded();
    let binding = binding(&saved, true, "b");
    let command = command(&binding, "stale-review");
    let receipt = match ledger.claim_command(&command, &accepted(&command)).unwrap() {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("fixture receipt must be new"),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "stale-review:rejected".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionPlanMismatch".into(),
        payload: json!({}),
    };
    assert_eq!(
        ledger
            .settle_rejected_provider_prompt_provision(&command, &receipt, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Rejected
    );
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "awaiting_readiness"
    );
}

fn binding(
    saved: &gent_types::AgentChatPromptSaved,
    consent_granted: bool,
    expected_digest: &str,
) -> ProviderPromptProvisionCommandBinding {
    ProviderPromptProvisionCommandBinding {
        prompt: ProviderPromptProvisionBinding {
            prompt_receipt_id: saved.receipt.receipt_id.clone(),
            conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
            run_id: saved.run_id.clone(),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted,
            reviewed_plan_digest: "a".repeat(64),
        },
        expected_reviewed_plan_digest: expected_digest.repeat(64),
        package: ProviderPromptProvisionPackageBinding {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
        },
    }
}

fn command(binding: &ProviderPromptProvisionCommandBinding, suffix: &str) -> Command {
    Command {
        receipt_id: ReceiptId(format!("receipt-{suffix}")),
        idempotency_key: format!("key-{suffix}"),
        host_epoch: HostEpoch(1),
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(binding).unwrap(),
    }
}

fn accepted(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "privatePromptProvisionAccepted".into(),
        payload: command.payload.clone(),
    }
}
