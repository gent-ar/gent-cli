use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationLedger};
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    WorkspaceRecord,
};

use super::{AsyncOrdinaryLifecycleHost, ClaurstPromptLifecycle};

fn waiting_prompt(disposition: AgentChatPromptDisposition) -> (SqliteLedger, AgentChatPromptSaved) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-a".into()),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "Reply with exactly the word PING".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    (ledger, saved)
}

async fn recovered(
    ledger: &SqliteLedger,
    bridge: &FakePrivateClaurstBridge,
) -> ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge> {
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger.clone(),
        bridge.clone(),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    lifecycle
}

fn phase(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
    ledger
        .find_turn(&saved.message.turn_id)
        .unwrap()
        .unwrap()
        .phase
}

#[tokio::test]
async fn stopping_a_claurst_turn_still_waiting_for_the_runtime_interrupts_it_before_it_starts() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    lifecycle
        .interrupt_claurst_turn(&saved.run_id.0, &saved.message.turn_id)
        .await
        .unwrap();
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Interrupted);
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

#[tokio::test]
async fn stopping_a_claurst_run_with_nothing_started_interrupts_its_waiting_turn() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Send);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    lifecycle
        .interrupt_claurst_run(&saved.run_id.0)
        .await
        .unwrap();
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Interrupted);
    assert!(!lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

#[tokio::test]
async fn stopping_a_claurst_run_leaves_its_queued_prompts_to_their_own_cancel() {
    let (ledger, saved) = waiting_prompt(AgentChatPromptDisposition::Queue);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    assert!(
        lifecycle
            .interrupt_claurst_run(&saved.run_id.0)
            .await
            .is_err()
    );
    assert_eq!(phase(&ledger, &saved), DurableTurnPhase::Active);
}
