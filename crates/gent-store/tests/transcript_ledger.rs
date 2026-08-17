use gent_ports::{AgentChatLedger, AgentChatPromptLedger, AgentChatReadLedger, TranscriptLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptKind, ReceiptId,
};

fn ledger() -> (SqliteLedger, AgentChatConversationId, String, String) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-1".into());
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("receipt-conversation".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            run_id: AgentChatRunId("run-1".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("receipt-prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
    (
        ledger,
        conversation_id,
        prompt.message.turn_id,
        prompt.run_id.0,
    )
}

fn append(event_id: &str, turn_id: &str, run_id: &str, text: &str) -> NormalizedTranscriptAppend {
    NormalizedTranscriptAppend {
        event_id: event_id.into(),
        turn_id: turn_id.into(),
        run_id: run_id.into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: text.into(),
        is_partial: false,
    }
}

#[test]
fn append_assigns_conversation_cursor_and_exact_retries_are_idempotent() {
    let (ledger, conversation_id, turn_id, run_id) = ledger();
    let event = append("event-1", &turn_id, &run_id, "first");

    let first = ledger
        .append_normalized_transcript(&conversation_id, &event)
        .unwrap();
    let retry = ledger
        .append_normalized_transcript(&conversation_id, &event)
        .unwrap();

    assert_eq!(first, retry);
    assert_eq!(first.cursor, 1);
    let mut conflict = event;
    conflict.text = "changed".into();
    assert!(
        ledger
            .append_normalized_transcript(&conversation_id, &conflict)
            .is_err()
    );
    assert_eq!(
        ledger
            .normalized_transcript_page(&conversation_id, 0, 10)
            .unwrap()
            .events
            .len(),
        1
    );
}

#[test]
fn pages_are_bounded_cursor_ordered_and_available_through_the_read_port() {
    let (ledger, conversation_id, turn_id, run_id) = ledger();
    for (event_id, text) in [("event-1", "one"), ("event-2", "two"), ("event-3", "three")] {
        ledger
            .append_normalized_transcript(
                &conversation_id,
                &append(event_id, &turn_id, &run_id, text),
            )
            .unwrap();
    }

    let first = ledger
        .normalized_transcript_page(&conversation_id, 0, 2)
        .unwrap();
    assert_eq!(
        first
            .events
            .iter()
            .map(|item| item.cursor)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(first.next_after_cursor, Some(2));
    let second = ledger
        .read_agent_chat_transcript("conversation-1", first.next_after_cursor, 2)
        .unwrap();
    assert_eq!(
        second
            .events
            .iter()
            .map(|item| &item.text)
            .collect::<Vec<_>>(),
        vec!["three"]
    );
    assert_eq!(second.next_after_cursor, None);
    assert!(
        ledger
            .normalized_transcript_page(&conversation_id, 0, 101)
            .is_err()
    );
}

#[test]
fn read_port_derives_public_selection_and_run_hierarchy_without_provider_session_data() {
    let (ledger, _, _, _) = ledger();
    let detail = ledger.read_agent_chat_detail("conversation-1").unwrap();

    assert_eq!(detail.summary.selection.provider, AgentChatProvider::Codex);
    assert_eq!(detail.runs.len(), 1);
    assert_eq!(detail.runs[0].run_id, "run-1");
    assert!(ledger.read_agent_chat_summary("missing").is_err());
}
