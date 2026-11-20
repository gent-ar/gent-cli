use gent_ports::{AgentChatLedger, ConversationLedger, TranscriptLedger, TurnFollowReader};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId, TurnRecord,
};

#[test]
fn exact_turn_page_never_leaks_a_sibling_turn() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.create_agent_chat_conversation(&create()).unwrap();
    for turn_id in ["turn-1", "turn-2"] {
        ledger
            .create_turn(&TurnRecord {
                turn_id: turn_id.into(),
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                sequence: if turn_id == "turn-1" { 1 } else { 2 },
                phase: DurableTurnPhase::Active,
            })
            .unwrap();
        ledger
            .append_normalized_transcript(
                &AgentChatConversationId("conversation".into()),
                &NormalizedTranscriptAppend {
                    event_id: format!("event-{turn_id}"),
                    turn_id: turn_id.into(),
                    run_id: "run".into(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: turn_id.into(),
                    is_partial: false,
                },
            )
            .unwrap();
    }
    let page = ledger
        .turn_follow_page("conversation", "run", "turn-1", 0, 1)
        .unwrap();
    assert_eq!(page.turn.turn_id, "turn-1");
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].turn_id, "turn-1");
    assert!(page.next_after_cursor.is_none());
    assert!(
        ledger
            .turn_follow_page("conversation", "run", "turn-2", 0, 1)
            .is_ok()
    );
    assert!(
        ledger
            .turn_follow_page("conversation", "run", "other", 0, 1)
            .is_err()
    );
}

#[test]
fn a_turn_follow_reads_every_transcript_kind_the_ledger_stores() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.create_agent_chat_conversation(&create()).unwrap();
    ledger
        .create_turn(&TurnRecord {
            turn_id: "turn-1".into(),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    let kinds = [
        NormalizedTranscriptKind::UserMessage,
        NormalizedTranscriptKind::AssistantMessage,
        NormalizedTranscriptKind::Thinking,
        NormalizedTranscriptKind::ToolActivity,
        NormalizedTranscriptKind::Notice,
        NormalizedTranscriptKind::Plan,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        ledger
            .append_normalized_transcript(
                &AgentChatConversationId("conversation".into()),
                &NormalizedTranscriptAppend {
                    event_id: format!("event-{index}"),
                    turn_id: "turn-1".into(),
                    run_id: "run".into(),
                    kind,
                    text: "text".into(),
                    is_partial: false,
                },
            )
            .unwrap();
    }
    let page = ledger
        .turn_follow_page("conversation", "run", "turn-1", 0, 10)
        .unwrap();
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        kinds
    );
}

fn create() -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: ReceiptId("receipt".into()),
        idempotency_key: "key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        },
    }
}
