use super::{SqliteLedger, tests};
use gent_ports::{Ledger, PrivateProviderPromptProvisionLedger, ReceiptClaim};
use gent_types::{
    AgentChatConversationId, Command, Event, HostEpoch, ProviderPromptProvisionBinding,
    ProviderPromptProvisionCommandBinding, ProviderPromptProvisionPackageBinding, ReceiptId,
    ReceiptStatus,
};
use serde_json::json;

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
    let terminal = Event {
        cursor: 0,
        event_id: "refusal:consent".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "privatePromptProvisionConsentRequired".into(),
        payload: command.payload.clone(),
    };
    assert_eq!(
        ledger
            .reject_verified_provider_prompt_provision(&command, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Rejected
    );
    assert_eq!(
        ledger
            .reject_verified_provider_prompt_provision(&command, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Rejected
    );
    assert_eq!(accepted_event_count(&ledger, &command.idempotency_key), 0);
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "awaiting_readiness"
    );
}

#[test]
fn a_fresh_rejection_reopens_the_unchanged_held_prompt() {
    let (ledger, saved) = tests::seeded();
    let binding = binding(&saved, false, "a");
    for suffix in ["first", "second"] {
        let command = command(&binding, suffix);
        let terminal = Event {
            cursor: 0,
            event_id: format!("{suffix}:rejected"),
            receipt_id: command.receipt_id.clone(),
            host_epoch: command.host_epoch,
            kind: "privatePromptProvisionConsentRequired".into(),
            payload: command.payload.clone(),
        };
        assert_eq!(
            ledger
                .reject_verified_provider_prompt_provision(&command, &terminal, &binding)
                .unwrap()
                .status,
            ReceiptStatus::Rejected
        );
    }
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "awaiting_readiness"
    );
}

#[test]
fn rejection_event_conflict_rolls_back_the_terminal_receipt() {
    let (ledger, saved) = tests::seeded();
    let binding = binding(&saved, false, "a");
    let command = command(&binding, "conflict");
    let terminal = Event {
        cursor: 0,
        event_id: existing_event_id(&ledger),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "privatePromptProvisionConsentRequired".into(),
        payload: command.payload.clone(),
    };
    assert!(
        ledger
            .reject_verified_provider_prompt_provision(&command, &terminal, &binding)
            .is_err()
    );
    assert_eq!(receipt_count(&ledger, &command.idempotency_key), 0);
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
    let terminal = Event {
        cursor: 0,
        event_id: "stale-review:rejected".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "privatePromptProvisionPlanMismatch".into(),
        payload: command.payload.clone(),
    };
    assert_eq!(
        ledger
            .reject_verified_provider_prompt_provision(&command, &terminal, &binding)
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
        release_artifact_digest_sha256: "d".repeat(64),
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

fn accepted_event_count(ledger: &SqliteLedger, key: &str) -> usize {
    ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = 'privatePromptProvisionAccepted' AND receipt_id = (SELECT receipt_id FROM receipts WHERE idempotency_key = ?1)",
            [key],
            |row| row.get::<_, usize>(0),
        )
        .unwrap()
}

fn existing_event_id(ledger: &SqliteLedger) -> String {
    let command = Command {
        receipt_id: ReceiptId("conflict-receipt".into()),
        idempotency_key: "conflict-key".into(),
        host_epoch: HostEpoch(1),
        kind: "test".into(),
        payload: json!({}),
    };
    let event_id = "prompt-provision-event-conflict".to_owned();
    ledger
        .claim_command(
            &command,
            &Event {
                cursor: 0,
                event_id: event_id.clone(),
                receipt_id: command.receipt_id.clone(),
                host_epoch: command.host_epoch,
                kind: "testAccepted".into(),
                payload: command.payload.clone(),
            },
        )
        .unwrap();
    event_id
}

fn receipt_count(ledger: &SqliteLedger, key: &str) -> usize {
    ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM receipts WHERE idempotency_key = ?1",
            [key],
            |row| row.get::<_, usize>(0),
        )
        .unwrap()
}
