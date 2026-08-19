use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger, PrivateProviderPromptProvisionLedger,
    ReceiptClaim,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, Command, Event, HostEpoch, ProviderPromptProvisionBinding,
    ProviderPromptProvisionCommandBinding, ProviderPromptProvisionPackageBinding, Receipt,
    ReceiptId, ReceiptStatus, WorkspaceRecord,
};
use serde_json::json;

use super::SqliteLedger;

#[test]
fn unprovable_settlement_terminally_fences_the_exact_reserved_prompt() {
    let (ledger, message_id, command, receipt, binding) = reserved_prompt();
    let terminal = Event {
        cursor: 0,
        event_id: "unprovable".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionUnprovable".into(),
        payload: json!({}),
    };
    assert_eq!(
        ledger
            .settle_unprovable_provider_prompt_provision(&command, &receipt, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Unprovable
    );
    assert_eq!(dispatch_state(&ledger, &message_id), "unprovable");
    assert_eq!(
        ledger
            .settle_unprovable_provider_prompt_provision(&command, &receipt, &terminal, &binding)
            .unwrap()
            .status,
        ReceiptStatus::Unprovable
    );
}

#[test]
fn proven_pre_effect_failure_reopens_the_exact_reserved_prompt_idempotently() {
    let (ledger, message_id, command, receipt, binding) = reserved_prompt();
    let terminal = failure(&command, &receipt, "pre-effect-failed");
    let rejected = ledger
        .reject_pre_effect_verified_provider_prompt_provision(
            &command, &receipt, &terminal, &binding,
        )
        .unwrap();
    assert_eq!(rejected.status, ReceiptStatus::Rejected);
    assert_eq!(dispatch_state(&ledger, &message_id), "awaiting_readiness");
    assert_eq!(
        ledger
            .reject_pre_effect_verified_provider_prompt_provision(
                &command, &rejected, &terminal, &binding,
            )
            .unwrap()
            .status,
        ReceiptStatus::Rejected
    );
}

#[test]
fn pre_effect_failure_rejects_conflicting_terminal_or_status() {
    let (ledger, message_id, command, receipt, binding) = reserved_prompt();
    let terminal = failure(&command, &receipt, "pre-effect-failed");
    ledger
        .reject_pre_effect_verified_provider_prompt_provision(
            &command, &receipt, &terminal, &binding,
        )
        .unwrap();
    let conflicting = failure(&command, &receipt, "different-terminal");
    assert!(
        ledger
            .reject_pre_effect_verified_provider_prompt_provision(
                &command,
                &receipt,
                &conflicting,
                &binding,
            )
            .is_err()
    );
    assert_eq!(dispatch_state(&ledger, &message_id), "awaiting_readiness");

    let (ledger, message_id, command, receipt, binding) = reserved_prompt();
    let terminal = Event {
        cursor: 0,
        event_id: "unprovable".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionUnprovable".into(),
        payload: json!({}),
    };
    ledger
        .settle_unprovable_provider_prompt_provision(&command, &receipt, &terminal, &binding)
        .unwrap();
    assert!(
        ledger
            .reject_pre_effect_verified_provider_prompt_provision(
                &command,
                &receipt,
                &failure(&command, &receipt, "pre-effect-failed"),
                &binding,
            )
            .is_err()
    );
    assert_eq!(dispatch_state(&ledger, &message_id), "unprovable");
}

#[test]
fn conflicting_failure_event_rolls_back_receipt_and_reservation() {
    let (ledger, message_id, command, receipt, binding) = reserved_prompt();
    assert!(
        ledger
            .reject_pre_effect_verified_provider_prompt_provision(
                &command,
                &receipt,
                &failure(&command, &receipt, "accepted"),
                &binding,
            )
            .is_err()
    );
    assert_eq!(dispatch_state(&ledger, &message_id), "provisioning");
}

fn failure(command: &Command, receipt: &Receipt, event_id: &str) -> Event {
    Event {
        cursor: 0,
        event_id: event_id.into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionFailed".into(),
        payload: command.payload.clone(),
    }
}

fn reserved_prompt() -> (
    SqliteLedger,
    String,
    Command,
    Receipt,
    ProviderPromptProvisionCommandBinding,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
        })
        .unwrap();
    let binding = ProviderPromptProvisionCommandBinding {
        prompt: ProviderPromptProvisionBinding {
            prompt_receipt_id: prompt.receipt.receipt_id.clone(),
            conversation_id: conversation,
            run_id: prompt.run_id.clone(),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
        },
        expected_reviewed_plan_digest: "a".repeat(64),
        package: ProviderPromptProvisionPackageBinding {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
        },
    };
    let command = Command {
        receipt_id: ReceiptId("provision-receipt".into()),
        idempotency_key: "provision-key".into(),
        host_epoch: HostEpoch(1),
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(&binding).unwrap(),
    };
    let receipt = match ledger
        .claim_command(
            &command,
            &Event {
                cursor: 0,
                event_id: "accepted".into(),
                receipt_id: command.receipt_id.clone(),
                host_epoch: HostEpoch(1),
                kind: "accepted".into(),
                payload: json!({}),
            },
        )
        .unwrap()
    {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("fixture command must be new"),
    };
    ledger
        .reserve_verified_provider_prompt_provision(&command, &binding)
        .unwrap();
    (ledger, prompt.message.message_id, command, receipt, binding)
}

fn dispatch_state(ledger: &SqliteLedger, message_id: &str) -> String {
    ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM agent_chat_prompt_dispatches WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
}
