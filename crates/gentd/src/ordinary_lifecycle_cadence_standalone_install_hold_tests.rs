use std::sync::Arc;

use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, ConversationLedger, Ledger, PrivateProviderPromptProvisionLedger,
    ReceiptClaim,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, Command, ConversationActivityFact, DurableTurnPhase, Event,
    HostEpoch, PromptHoldReason, ProviderPromptProvisionBinding,
    ProviderPromptProvisionCommandBinding, ProviderPromptProvisionPackageBinding, Receipt,
    ReceiptId, TurnPhase, WorkspaceRecord,
};

use super::super::{StandalonePromptRelease, StandalonePromptReleaseOutcome, StandaloneReadiness};
use crate::agent_chat_api::PromptWake;

#[derive(Debug)]
struct NotInstalled;

impl crate::standalone_provider_readiness::StandalonePublicProviderReadiness for NotInstalled {
    fn is_ready(&self, _: AgentChatProvider) -> Result<bool, String> {
        Ok(false)
    }
}

struct Held {
    conversation: String,
    run: String,
    receipt: String,
    turn_id: String,
}

fn held(ledger: &SqliteLedger, readiness: &StandaloneReadiness, name: &'static str) -> Held {
    let conversation = format!("conversation-{name}");
    let run = format!("run-{name}");
    let receipt = format!("receipt-{name}");
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("create-{name}")),
                idempotency_key: format!("create-{name}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation.clone()),
                run_id: AgentChatRunId(run.clone()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
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
    let turn_id = send(ledger, &conversation, &receipt).message.turn_id;
    let value = Held {
        conversation,
        run,
        receipt,
        turn_id,
    };
    assert_eq!(
        readiness.release(&wake(&value)).unwrap(),
        StandalonePromptReleaseOutcome::Held
    );
    value
}

fn send(
    ledger: &SqliteLedger,
    conversation: &str,
    receipt: &str,
) -> gent_types::AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{receipt}")),
            receipt_id: ReceiptId(receipt.into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(conversation.into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "install then answer".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap()
}

fn wake(held: &Held) -> PromptWake {
    PromptWake {
        conversation_id: AgentChatConversationId(held.conversation.clone()),
        run_id: AgentChatRunId(held.run.clone()),
        receipt_id: ReceiptId(held.receipt.clone()),
        disposition: AgentChatPromptDisposition::Send,
    }
}

fn setup() -> (tempfile::TempDir, SqliteLedger, StandaloneReadiness) {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let readiness = StandaloneReadiness::new_with_public_readiness(
        ledger.clone(),
        HostEpoch(1),
        None,
        Arc::new(NotInstalled),
    );
    (directory, ledger, readiness)
}

fn restored(ledger: &SqliteLedger, held: &Held) -> Vec<ConversationActivityFact> {
    ledger
        .read_conversation_activity_page(&held.conversation, &held.run, 0, 50)
        .unwrap()
        .facts
}

fn projected(ledger: &SqliteLedger, held: &Held) -> Vec<ConversationActivityFact> {
    ledger
        .agent_chat_projection_page(&AgentChatConversationId(held.conversation.clone()), 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "activity")
        .map(|event| serde_json::from_value(event.payload["activity"].clone()).unwrap())
        .collect()
}

fn count(facts: &[ConversationActivityFact], name: &str) -> usize {
    facts
        .iter()
        .filter(|fact| serde_json::to_value(fact).unwrap()["type"] == name)
        .count()
}

fn phase(ledger: &SqliteLedger, held: &Held) -> DurableTurnPhase {
    ledger.find_turn(&held.turn_id).unwrap().unwrap().phase
}

fn assert_cancelled(ledger: &SqliteLedger, held: &Held) {
    assert_eq!(phase(ledger, held), DurableTurnPhase::Cancelled);
    for facts in [restored(ledger, held), projected(ledger, held)] {
        assert_eq!(count(&facts, "promptHeld"), 1);
        assert_eq!(count(&facts, "promptCanceled"), 1);
        assert_eq!(count(&facts, "terminal"), 1);
        assert!(facts.iter().any(|fact| matches!(
            fact,
            ConversationActivityFact::Terminal {
                phase: TurnPhase::Cancelled,
                ..
            }
        )));
    }
}

fn assert_still_held(ledger: &SqliteLedger, held: &Held) {
    assert_eq!(phase(ledger, held), DurableTurnPhase::Active);
    let facts = restored(ledger, held);
    assert!(facts.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::PromptHeld {
            reason: PromptHoldReason::ProviderInstall,
            ..
        }
    )));
    assert_eq!(count(&facts, "promptCanceled"), 0);
    assert_eq!(count(&facts, "terminal"), 0);
}

fn reserve_install(
    ledger: &SqliteLedger,
    held: &Held,
) -> (ProviderPromptProvisionCommandBinding, Command, Receipt) {
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
        receipt_id: ReceiptId(format!("provision-{}", held.receipt)),
        idempotency_key: format!("provision-{}", held.receipt),
        host_epoch: HostEpoch(1),
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(&binding).unwrap(),
    };
    let accepted = Event {
        cursor: 0,
        event_id: format!("provision-{}-accepted", held.receipt),
        receipt_id: command.receipt_id.clone(),
        host_epoch: HostEpoch(1),
        kind: "accepted".into(),
        payload: serde_json::json!({}),
    };
    let ReceiptClaim::Accepted(receipt) = ledger.claim_command(&command, &accepted).unwrap() else {
        panic!("a new install command is accepted");
    };
    ledger
        .reserve_verified_provider_prompt_provision(&command, &binding)
        .unwrap();
    (binding, command, receipt)
}

#[test]
fn canceling_one_install_hold_leaves_the_other_waiting_and_its_conversation_usable() {
    let (_directory, ledger, readiness) = setup();
    let first = held(&ledger, &readiness, "first");
    let second = held(&ledger, &readiness, "second");

    assert!(readiness.cancel_held_prompts(&second.run).unwrap());

    assert_cancelled(&ledger, &second);
    assert_still_held(&ledger, &first);
    send(&ledger, &second.conversation, "second-again");
    assert!(!readiness.cancel_held_prompts(&second.run).unwrap());
}

#[test]
fn a_running_install_cannot_be_cancelled_but_its_prompt_is_cancellable_once_it_is_held_again() {
    let (_directory, ledger, readiness) = setup();
    let first = held(&ledger, &readiness, "first");
    let second = held(&ledger, &readiness, "second");

    assert!(readiness.cancel_held_prompts(&first.run).unwrap());
    assert_cancelled(&ledger, &first);
    assert_still_held(&ledger, &second);

    let (binding, command, receipt) = reserve_install(&ledger, &second);
    assert_eq!(
        readiness.cancel_held_prompts(&second.run),
        Err("the held prompt's provider install is already running".to_owned())
    );
    assert_still_held(&ledger, &second);

    let failed = Event {
        cursor: 0,
        event_id: "provision-second-failed".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionFailed".into(),
        payload: command.payload.clone(),
    };
    ledger
        .reject_pre_effect_verified_provider_prompt_provision(&command, &receipt, &failed, &binding)
        .unwrap();
    assert!(readiness.cancel_held_prompts(&second.run).unwrap());
    assert_cancelled(&ledger, &second);
}
