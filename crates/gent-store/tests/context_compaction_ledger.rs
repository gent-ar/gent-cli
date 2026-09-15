use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ContextCompactionLedger, Ledger,
    TranscriptLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CONTEXT_COMPACTION_EVENT_KIND, ContextCompactionFact,
    ContextCompactionFailure, ContextCompactionPlan, ContextCompactionTrigger,
    ContextCoverageDigest, HostEpoch, NormalizedTranscriptKind, PROVIDER_CONTEXT_COMPACTED_NOTICE,
    ReceiptId, WorkspaceRecord,
};

fn conversation(ledger: &SqliteLedger) -> Vec<String> {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation".into()),
                idempotency_key: "conversation".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    (1..=2)
        .map(|index| {
            ledger
                .save_agent_chat_prompt(&AgentChatPromptCreate {
                    request_id: AgentChatRequestId(format!("prompt-{index}")),
                    receipt_id: ReceiptId(format!("prompt-{index}")),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    disposition: AgentChatPromptDisposition::Queue,
                    text: format!("message {index}"),
                    attachment_ids: vec![],
                    tool_source_ids: vec![],
                })
                .unwrap()
                .message
                .turn_id
        })
        .collect()
}

fn plan(turn_id: &str) -> ContextCompactionPlan {
    ContextCompactionPlan {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: turn_id.into(),
        trigger: ContextCompactionTrigger::Command,
        covers_through_ordinal: 1,
        covered_digest_sha256: ContextCoverageDigest::new(false).current(),
        imports_covered: false,
        previous_summary: None,
        items: vec![],
        omitted_source_items: 0,
    }
}

#[test]
fn a_compaction_fact_and_its_notice_commit_together_and_retries_are_exact() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let turns = conversation(&ledger);
    let fact = plan(&turns[0]).compacted("The user planted LARK-7.".into(), 12);

    ledger
        .record_context_compaction(&fact, HostEpoch(1))
        .unwrap();
    ledger
        .record_context_compaction(&fact, HostEpoch(1))
        .unwrap();

    let event = ledger.find_event(&fact.event_id()).unwrap().unwrap();
    assert_eq!(event.kind, CONTEXT_COMPACTION_EVENT_KIND);
    let notices = ledger
        .normalized_transcript_page(&AgentChatConversationId("conversation-1".into()), 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == NormalizedTranscriptKind::Notice)
        .map(|event| (event.turn_id, event.text))
        .collect::<Vec<_>>();
    assert_eq!(
        notices,
        [(
            turns[0].clone(),
            PROVIDER_CONTEXT_COMPACTED_NOTICE.to_owned()
        )]
    );
    let conflicting = plan(&turns[0]).compacted("another summary".into(), 3);
    assert!(
        ledger
            .record_context_compaction(&conflicting, HostEpoch(1))
            .is_err()
    );
    assert_eq!(
        ledger.context_compactions("conversation-1", 8).unwrap(),
        [fact]
    );
}

#[test]
fn facts_read_newest_first_per_conversation_and_stale_epochs_write_nothing() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let turns = conversation(&ledger);
    let first = plan(&turns[0]).compacted("first".into(), 1);
    let second = plan(&turns[1]).failed(ContextCompactionFailure::OutputLimit);
    ledger
        .record_context_compaction(&first, HostEpoch(1))
        .unwrap();
    assert!(
        ledger
            .record_context_compaction(&second, HostEpoch(2))
            .is_err()
    );
    assert!(ledger.find_event(&second.event_id()).unwrap().is_none());
    ledger
        .record_context_compaction(&second, HostEpoch(1))
        .unwrap();

    let facts = ledger.context_compactions("conversation-1", 8).unwrap();
    assert_eq!(facts, [second, first.clone()]);
    assert_eq!(
        ledger
            .context_compactions("conversation-1", 1)
            .unwrap()
            .len(),
        1
    );
    assert!(ledger.context_compactions("other", 8).unwrap().is_empty());
    assert!(matches!(facts[1], ContextCompactionFact::Compacted { .. }));
}

#[test]
fn an_invalid_fact_is_refused_before_any_row_is_written() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let turns = conversation(&ledger);
    let invalid = plan(&turns[0]).compacted(String::new(), 0);
    assert!(
        ledger
            .record_context_compaction(&invalid, HostEpoch(1))
            .is_err()
    );
    assert!(
        ledger
            .context_compactions("conversation-1", 8)
            .unwrap()
            .is_empty()
    );
}
