use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatWorkspaceLedger, ConversationActivityLedger, ConversationLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, Command, ConversationActivityFact, DurableTurnPhase, Event,
    HostEpoch, PromptAdmissionExit, PromptHoldReason, ProviderPromptReadinessBinding,
    ProviderPromptReadinessFailureBinding, ReceiptId, TurnPhase, WorkspaceRecord,
};

const CONVERSATION: &str = "conversation-1";
const RUN: &str = "run-1";

fn held() -> (SqliteLedger, gent_types::AgentChatPromptSaved) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId(CONVERSATION.into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId(RUN.into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-8b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "explain this repo".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .hold_agent_chat_prompt_for_admission(
            &saved.receipt.receipt_id,
            HostEpoch(1),
            PromptHoldReason::ModelDownload,
        )
        .unwrap();
    (ledger, saved)
}

fn restored(ledger: &SqliteLedger) -> Vec<ConversationActivityFact> {
    ledger
        .read_conversation_activity_page(CONVERSATION, RUN, 0, 50)
        .unwrap()
        .facts
}

fn projected(ledger: &SqliteLedger) -> Vec<ConversationActivityFact> {
    ledger
        .agent_chat_projection_page(&AgentChatConversationId(CONVERSATION.into()), 0, 50)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "activity")
        .map(|event| serde_json::from_value(event.payload["activity"].clone()).unwrap())
        .collect()
}

fn snapshot(ledger: &SqliteLedger) -> Vec<ConversationActivityFact> {
    ledger
        .agent_chat_projection_tail(&AgentChatConversationId(CONVERSATION.into()), 50, 50)
        .unwrap()
        .activity
        .into_iter()
        .map(|event| serde_json::from_value(event.payload["activity"].clone()).unwrap())
        .collect()
}

fn count(facts: &[ConversationActivityFact], name: &str) -> usize {
    facts
        .iter()
        .filter(|fact| serde_json::to_value(fact).unwrap()["type"] == name)
        .count()
}

fn readiness(
    saved: &gent_types::AgentChatPromptSaved,
) -> (ProviderPromptReadinessBinding, Command, Event) {
    let binding = ProviderPromptReadinessBinding {
        prompt_receipt_id: saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(CONVERSATION.into()),
        run_id: saved.run_id.clone(),
        provider: AgentChatProvider::Claurst,
    };
    let payload = serde_json::to_value(&binding).unwrap();
    let command = Command {
        receipt_id: ReceiptId("readiness-receipt".into()),
        idempotency_key: "readiness-key".into(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReadiness".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "provider-ready".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReady".into(),
        payload,
    };
    (binding, command, terminal)
}

fn admission_exit(
    saved: &gent_types::AgentChatPromptSaved,
    exit: PromptAdmissionExit,
) -> (ProviderPromptReadinessFailureBinding, Command, Event) {
    let binding = ProviderPromptReadinessFailureBinding {
        prompt_receipt_id: saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(CONVERSATION.into()),
        run_id: saved.run_id.clone(),
        provider: AgentChatProvider::Claurst,
        exit,
        reason: "the download was canceled".into(),
    };
    let payload = serde_json::to_value(&binding).unwrap();
    let command = Command {
        receipt_id: ReceiptId("exit-receipt".into()),
        idempotency_key: "exit-key".into(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReadinessFailure".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "provider-readiness-failed".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReadinessFailed".into(),
        payload,
    };
    (binding, command, terminal)
}

#[test]
fn a_held_prompt_is_replayable_from_the_durable_projection_with_its_typed_reason() {
    let (ledger, saved) = held();
    let fact = restored(&ledger)
        .into_iter()
        .find(|fact| matches!(fact, ConversationActivityFact::PromptHeld { .. }))
        .expect("the hold is durable");
    let value = serde_json::to_value(&fact).unwrap();
    assert_eq!(value["type"], "promptHeld");
    assert_eq!(value["conversationId"], CONVERSATION);
    assert_eq!(value["runId"], RUN);
    assert_eq!(value["turnId"], saved.message.turn_id);
    assert_eq!(value["hostEpoch"], 1);
    assert_eq!(value["messageId"], saved.message.message_id);
    assert_eq!(value["receiptId"], "prompt-receipt");
    assert_eq!(value["reason"], "modelDownload");
    assert!(value["cursor"].as_u64().unwrap() > 0);
    assert!(projected(&ledger).contains(&fact));
    assert!(snapshot(&ledger).contains(&fact));
}

#[test]
fn repeating_the_hold_never_duplicates_the_fact_a_late_client_replays() {
    let (ledger, saved) = held();
    ledger
        .hold_agent_chat_prompt_for_admission(
            &saved.receipt.receipt_id,
            HostEpoch(1),
            PromptHoldReason::ModelDownload,
        )
        .unwrap();
    assert_eq!(count(&restored(&ledger), "promptHeld"), 1);
    assert_eq!(count(&projected(&ledger), "promptHeld"), 1);
}

#[test]
fn releasing_a_held_prompt_clears_the_hold_exactly_once_and_starts_no_turn_terminal() {
    let (ledger, saved) = held();
    let (binding, command, terminal) = readiness(&saved);
    ledger
        .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    ledger
        .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    let facts = restored(&ledger);
    assert_eq!(count(&facts, "promptReleased"), 1);
    assert_eq!(count(&facts, "promptCanceled"), 0);
    assert_eq!(count(&facts, "terminal"), 0);
    assert_eq!(count(&projected(&ledger), "promptReleased"), 1);
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Active
    );
}

#[test]
fn canceling_a_held_prompt_settles_it_as_cancelled_rather_than_a_provider_failure() {
    let (ledger, saved) = held();
    let (binding, command, terminal) = admission_exit(&saved, PromptAdmissionExit::Cancelled);
    ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    let facts = restored(&ledger);
    assert_eq!(count(&facts, "promptCanceled"), 1);
    assert_eq!(count(&facts, "promptReleased"), 0);
    let terminal_fact = facts
        .iter()
        .find(|fact| matches!(fact, ConversationActivityFact::Terminal { .. }))
        .expect("the cancelled turn carries a terminal fact");
    assert_eq!(
        serde_json::to_value(terminal_fact).unwrap()["phase"],
        "cancelled"
    );
    assert!(matches!(
        terminal_fact,
        ConversationActivityFact::Terminal {
            phase: TurnPhase::Cancelled,
            ..
        }
    ));
    assert_eq!(count(&facts, "terminal"), 1);
    assert_eq!(count(&projected(&ledger), "terminal"), 1);
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Cancelled
    );
}

#[test]
fn a_readiness_failure_still_settles_the_held_prompt_as_failed() {
    let (ledger, saved) = held();
    let (binding, command, terminal) = admission_exit(&saved, PromptAdmissionExit::Failed);
    ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    let facts = restored(&ledger);
    assert!(facts.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::Terminal {
            phase: TurnPhase::Failed,
            ..
        }
    )));
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Failed
    );
}
