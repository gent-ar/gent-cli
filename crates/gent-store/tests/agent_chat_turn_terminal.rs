use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatQueuedPromptLedger, AgentChatWorkspaceLedger, ConversationActivityLedger,
    ConversationLedger, Ledger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ConversationActivityFact, DurableTurnPhase, HostEpoch,
    ReceiptId, TurnPhase, WorkspaceRecord,
};

const CONVERSATION: &str = "conversation-1";
const RUN: &str = "run-1";

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(CONVERSATION.into()),
                run_id: AgentChatRunId(RUN.into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
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
    ledger
}

fn prompt(
    ledger: &SqliteLedger,
    request_id: &str,
    disposition: AgentChatPromptDisposition,
) -> gent_types::AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request_id.into()),
            receipt_id: ReceiptId(format!("receipt-{request_id}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(CONVERSATION.into()),
            disposition,
            text: format!("prompt-{request_id}"),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            HostEpoch(1),
        )
        .unwrap();
    saved
}

fn restored_terminals(ledger: &SqliteLedger, turn_id: &str) -> Vec<TurnPhase> {
    ledger
        .read_conversation_activity_page(CONVERSATION, RUN, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| terminal_phase(&fact, turn_id))
        .collect()
}

fn live_terminals(ledger: &SqliteLedger, turn_id: &str) -> Vec<TurnPhase> {
    ledger
        .agent_chat_projection_page(&AgentChatConversationId(CONVERSATION.into()), 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "activity")
        .filter_map(|event| {
            serde_json::from_value::<ConversationActivityFact>(event.payload["activity"].clone())
                .ok()
        })
        .filter_map(|fact| terminal_phase(&fact, turn_id))
        .collect()
}

fn terminal_phase(fact: &ConversationActivityFact, turn_id: &str) -> Option<TurnPhase> {
    match fact {
        ConversationActivityFact::Terminal { scope, phase, .. } if scope.turn_id == turn_id => {
            Some(phase.clone())
        }
        _ => None,
    }
}

fn assert_settled(
    ledger: &SqliteLedger,
    turn_id: &str,
    durable: DurableTurnPhase,
    live: TurnPhase,
) {
    assert_eq!(
        ledger.find_turn(turn_id).unwrap().unwrap().phase,
        durable,
        "durable turn phase"
    );
    assert_eq!(
        restored_terminals(ledger, turn_id),
        vec![live.clone()],
        "restore"
    );
    assert_eq!(live_terminals(ledger, turn_id), vec![live], "live");
}

fn start(ledger: &SqliteLedger, saved: &gent_types::AgentChatPromptSaved) {
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
}

#[test]
fn a_completed_turn_reads_the_same_terminal_live_and_after_restore() {
    let ledger = ledger();
    let saved = prompt(&ledger, "completed", AgentChatPromptDisposition::Send);
    start(&ledger, &saved);
    ledger
        .settle_agent_chat_prompt_terminal(
            &saved.message.message_id,
            "daemon-a",
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Completed,
        TurnPhase::Ready,
    );
}

#[test]
fn an_interrupted_turn_reads_the_same_terminal_live_and_after_restore() {
    let ledger = ledger();
    let saved = prompt(&ledger, "interrupted", AgentChatPromptDisposition::Send);
    start(&ledger, &saved);
    ledger
        .settle_agent_chat_prompt_terminal(
            &saved.message.message_id,
            "daemon-a",
            HostEpoch(1),
            DurableTurnPhase::Interrupted,
        )
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Interrupted,
        TurnPhase::Interrupted,
    );
}

#[test]
fn a_failed_turn_reads_the_same_terminal_live_and_after_restore() {
    let ledger = ledger();
    let saved = prompt(&ledger, "failed", AgentChatPromptDisposition::Send);
    start(&ledger, &saved);
    ledger
        .settle_agent_chat_prompt_terminal(
            &saved.message.message_id,
            "daemon-a",
            HostEpoch(1),
            DurableTurnPhase::Failed,
        )
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Failed,
        TurnPhase::Failed,
    );
}

#[test]
fn a_prelaunch_failure_settles_with_a_terminal_fact_rather_than_a_bare_turn_row() {
    let ledger = ledger();
    let saved = prompt(&ledger, "prelaunch", AgentChatPromptDisposition::Send);
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    ledger
        .fail_agent_chat_prompt_prelaunch(
            &saved.message.message_id,
            "daemon-a",
            HostEpoch(1),
            "the provider executable is missing",
        )
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Failed,
        TurnPhase::Failed,
    );
}

#[test]
fn an_unprovable_launch_settles_with_a_terminal_fact_rather_than_a_bare_turn_row() {
    let ledger = ledger();
    let saved = prompt(&ledger, "unprovable", AgentChatPromptDisposition::Send);
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .mark_agent_chat_prompt_unprovable(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Failed,
        TurnPhase::Failed,
    );
}

#[test]
fn epoch_recovery_settles_an_abandoned_turn_with_a_terminal_fact() {
    let ledger = ledger();
    let saved = prompt(&ledger, "abandoned", AgentChatPromptDisposition::Send);
    start(&ledger, &saved);
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_settled(
        &ledger,
        &saved.message.turn_id,
        DurableTurnPhase::Failed,
        TurnPhase::Failed,
    );
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_eq!(restored_terminals(&ledger, &saved.message.turn_id).len(), 1);
}

#[test]
fn canceling_a_queued_prompt_settles_its_turn_as_cancelled_in_both_views() {
    let ledger = ledger();
    let active = prompt(&ledger, "active", AgentChatPromptDisposition::Send);
    let queued = prompt(&ledger, "queued", AgentChatPromptDisposition::Queue);
    start(&ledger, &active);
    ledger
        .cancel_queued_agent_chat_prompt(
            &ReceiptId("cancel-receipt".into()),
            HostEpoch(1),
            &AgentChatConversationId(CONVERSATION.into()),
            &queued.message.message_id,
        )
        .unwrap();
    assert_settled(
        &ledger,
        &queued.message.turn_id,
        DurableTurnPhase::Cancelled,
        TurnPhase::Cancelled,
    );
}
