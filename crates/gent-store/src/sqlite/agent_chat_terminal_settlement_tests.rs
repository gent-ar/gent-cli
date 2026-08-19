use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationLedger, TranscriptLedger,
    agent_chat_terminal_settlement::AgentChatTerminalSettlementReader,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

use super::SqliteLedger;

#[test]
fn terminal_settlement_survives_reopen_and_requires_both_durable_fences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-a".into()),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt".into(),
                    effort: AgentChatEffort::Medium,
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
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
        })
        .unwrap();
    assert!(settlement(&ledger, &saved.message.turn_id).is_none());
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            HostEpoch(1),
        )
        .unwrap();
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .settle_agent_chat_prompt_dispatch(&saved.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    assert!(settlement(&ledger, &saved.message.turn_id).is_none());
    ledger
        .replace_turn_phase(
            &saved.message.turn_id,
            DurableTurnPhase::Active,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    let event = ledger
        .append_normalized_transcript(
            &AgentChatConversationId("conversation-a".into()),
            &NormalizedTranscriptAppend {
                event_id: "terminal-output".into(),
                run_id: "run-a".into(),
                turn_id: saved.message.turn_id.clone(),
                kind: NormalizedTranscriptKind::AssistantMessage,
                text: "done".into(),
                is_partial: false,
            },
        )
        .unwrap();
    drop(ledger);
    let reopened = SqliteLedger::open(&path).unwrap();
    assert!(matches!(
        settlement(&reopened, &saved.message.turn_id),
        Some(value)
            if value.conversation_id == "conversation-a"
                && value.phase == DurableTurnPhase::Completed
                && value.cursor == event.cursor
                && value.host_epoch == HostEpoch(1)
    ));
}

fn settlement(
    ledger: &SqliteLedger,
    turn_id: &str,
) -> Option<gent_types::AgentChatTerminalSettlement> {
    ledger
        .read_agent_chat_terminal_settlement("run-a", turn_id)
        .unwrap()
}
