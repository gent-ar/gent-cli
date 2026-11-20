use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatSelectionLedger,
    AgentChatWorkspaceLedger, Ledger, PrivateProviderPromptProvisionLedger, ReceiptClaim,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch, Command, ContextPolicy,
    ConversationActivityFact, Event, HostEpoch, PromptHoldReason, ProviderInstallProvenance,
    ProviderPromptProvisionBinding, ProviderPromptProvisionCommandBinding,
    ProviderPromptProvisionPackageBinding, ProvisionedProviderInstallation,
    ProvisionedProviderLock, ReceiptId, RunVersionLock, WorkspaceRecord,
};

use super::super::super::{StandalonePromptRelease, StandalonePromptReleaseOutcome};
use super::{Held, count, restored, setup};
use crate::agent_chat_api::PromptWake;

fn selection(provider: AgentChatProvider, model: &str) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn install(ledger: &gent_store::SqliteLedger, held: &Held) {
    let binding = ProviderPromptProvisionCommandBinding {
        prompt: ProviderPromptProvisionBinding {
            prompt_receipt_id: ReceiptId(held.receipt.clone()),
            conversation_id: AgentChatConversationId(held.conversation.clone()),
            run_id: AgentChatRunId(held.run.clone()),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
        },
        expected_reviewed_plan_digest: "a".repeat(64),
        release_artifact_digest_sha256: "d".repeat(64),
        package: ProviderPromptProvisionPackageBinding {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
        },
    };
    let command = Command {
        receipt_id: ReceiptId("provision-carried".into()),
        idempotency_key: "provision-carried".into(),
        host_epoch: HostEpoch(1),
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(&binding).unwrap(),
    };
    let accepted = Event {
        cursor: 0,
        event_id: "provision-carried-accepted".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: HostEpoch(1),
        kind: "privatePromptProvisionAccepted".into(),
        payload: command.payload.clone(),
    };
    let ReceiptClaim::Accepted(receipt) = ledger.claim_command(&command, &accepted).unwrap() else {
        panic!("a new install command is accepted");
    };
    ledger
        .reserve_verified_provider_prompt_provision(&command, &binding)
        .unwrap();
    let installation = ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock {
            run_lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: "/private/codex".into(),
                file_identity: "identity".into(),
                digest_sha256: "a".repeat(64),
                version: "1.0.0".into(),
                compatibility_entry: "codex-1".into(),
            },
        },
        provenance: ProviderInstallProvenance {
            package_name: "@openai/codex".into(),
            package_version: "1.0.0".into(),
            package_integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
            node_runtime_digest_sha256: "c".repeat(64),
            release_artifact_digest_sha256: binding.release_artifact_digest_sha256.clone(),
            receipt_fingerprint_sha256: command.receipt_fingerprint_sha256(),
        },
    };
    let installed = Event {
        cursor: 0,
        event_id: "provision-carried-installed".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionInstalled".into(),
        payload: serde_json::json!({}),
    };
    ledger
        .settle_verified_provider_prompt_provision(
            &command,
            &receipt,
            &installation,
            &installed,
            &binding,
        )
        .unwrap();
}

#[test]
fn a_prompt_released_for_claude_waits_for_the_codex_install_after_a_switch_then_runs_on_codex() {
    let (_directory, ledger, readiness) = setup();
    let conversation = AgentChatConversationId("conversation-carried".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-carried".into()),
                idempotency_key: "create-carried".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run-claude".into()),
                selection: selection(AgentChatProvider::Claude, "haiku"),
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-carried".into()),
            receipt_id: ReceiptId("receipt-carried".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "answer after the switch".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch-carried".into()),
            idempotency_key: "switch-carried".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            parent_run_id: AgentChatRunId("run-claude".into()),
            run_id: AgentChatRunId("run-codex".into()),
            selection: selection(AgentChatProvider::Codex, "gpt-5.6"),
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();

    let carried = ledger
        .read_awaiting_agent_chat_prompts(Some("run-codex"), "", 128)
        .unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].message.message_id, saved.message.message_id);
    let held = Held {
        conversation: conversation.0.clone(),
        run: "run-codex".into(),
        receipt: saved.receipt.receipt_id.0.clone(),
        turn_id: saved.message.turn_id.clone(),
    };
    assert_eq!(
        readiness
            .release(&PromptWake {
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId(held.run.clone()),
                receipt_id: saved.receipt.receipt_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
            })
            .unwrap(),
        StandalonePromptReleaseOutcome::Held
    );
    assert!(restored(&ledger, &held).iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::PromptHeld {
            reason: PromptHoldReason::ProviderInstall,
            ..
        }
    )));
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );

    install(&ledger, &held);

    let delivered = ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(
        (delivered.message.message_id, delivered.run_id.0),
        (saved.message.message_id, held.run.clone())
    );
    assert_eq!(count(&restored(&ledger, &held), "promptReleased"), 1);
}
