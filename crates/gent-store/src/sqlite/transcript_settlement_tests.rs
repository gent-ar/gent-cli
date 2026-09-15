use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatReadLedger,
    AgentChatWorkspaceLedger, TranscriptLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

use super::super::SqliteLedger;

const CONVERSATION: &str = "conversation";

fn prompt(ledger: &SqliteLedger, receipt: &str) -> String {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{receipt}")),
            receipt_id: ReceiptId(receipt.into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(CONVERSATION.into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "count".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap()
        .message
        .turn_id
}

fn conversation() -> (tempfile::TempDir, SqliteLedger) {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(CONVERSATION.into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Ask,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    (directory, ledger)
}

fn append(
    ledger: &SqliteLedger,
    turn_id: &str,
    event_id: &str,
    kind: NormalizedTranscriptKind,
    text: &str,
    is_partial: bool,
) -> u64 {
    ledger
        .append_normalized_transcript(
            &AgentChatConversationId(CONVERSATION.into()),
            &NormalizedTranscriptAppend {
                event_id: event_id.into(),
                turn_id: turn_id.into(),
                run_id: "run".into(),
                kind,
                text: text.into(),
                is_partial,
            },
        )
        .unwrap()
        .cursor
}

fn settle(ledger: &SqliteLedger, turn_id: &str, phase: DurableTurnPhase) {
    let mut connection = ledger.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(
        super::super::turn_terminal::settle(&transaction, turn_id, HostEpoch(1), phase).unwrap()
    );
    transaction.commit().unwrap();
}

fn transcript(ledger: &SqliteLedger) -> Vec<NormalizedTranscriptEvent> {
    ledger
        .normalized_transcript_page(&AgentChatConversationId(CONVERSATION.into()), 0, 100)
        .unwrap()
        .events
}

fn projected(ledger: &SqliteLedger) -> Vec<gent_types::AgentChatProjectionEvent> {
    ledger
        .agent_chat_projection_page(&AgentChatConversationId(CONVERSATION.into()), 0, 100)
        .unwrap()
        .events
}

fn stream(
    ledger: &SqliteLedger,
    turn: &str,
    facts: &[(&str, NormalizedTranscriptKind, &str, bool)],
) {
    for (event_id, kind, text, is_partial) in facts {
        append(ledger, turn, event_id, *kind, text, *is_partial);
    }
}

fn visible(ledger: &SqliteLedger, turn: &str) -> Vec<(String, String, bool)> {
    transcript(ledger)
        .into_iter()
        .filter(|event| {
            event.turn_id == turn && event.kind != NormalizedTranscriptKind::UserMessage
        })
        .map(|event| (event.event_id, event.text, event.is_partial))
        .collect()
}

const ANSWER: NormalizedTranscriptKind = NormalizedTranscriptKind::AssistantMessage;
const THINKING: NormalizedTranscriptKind = NormalizedTranscriptKind::Thinking;

#[test]
fn page_reads_skip_partials_a_later_final_supersedes_while_committed_facts_stay_unchanged() {
    let (_directory, ledger) = conversation();
    let turn = prompt(&ledger, "prompt");
    stream(
        &ledger,
        &turn,
        &[
            ("p1", ANSWER, "Hel", true),
            ("p2", ANSWER, "lo", true),
            ("f1", ANSWER, "Hello", false),
            ("t1", THINKING, "hm", true),
            ("p3", ANSWER, "By", true),
        ],
    );
    assert_eq!(
        visible(&ledger, &turn),
        [
            ("f1".into(), "Hello".into(), false),
            ("t1".into(), "hm".into(), true),
            ("p3".into(), "By".into(), true)
        ]
    );
    let committed = projected(&ledger);
    stream(&ledger, &turn, &[("f2", ANSWER, "Bye", false)]);

    settle(&ledger, &turn, DurableTurnPhase::Completed);

    assert_eq!(
        visible(&ledger, &turn),
        [
            ("f1".into(), "Hello".into(), false),
            ("t1".into(), "hm".into(), true),
            ("f2".into(), "Bye".into(), false)
        ]
    );
    let after = projected(&ledger);
    assert_eq!(&after[..committed.len()], committed.as_slice());
    assert_eq!(after.len(), committed.len() + 2);
}

#[test]
fn settling_an_interrupted_turn_appends_its_unfinished_reply_after_every_committed_fact() {
    let (_directory, ledger) = conversation();
    let turn = prompt(&ledger, "prompt");
    stream(
        &ledger,
        &turn,
        &[
            ("f1", ANSWER, "1, 2", false),
            ("p2", ANSWER, "3, ", true),
            ("t1", THINKING, "   ", true),
        ],
    );
    let steer = prompt(&ledger, "steer");
    stream(&ledger, &turn, &[("p3", ANSWER, "4", true)]);
    let committed = projected(&ledger);

    settle(&ledger, &turn, DurableTurnPhase::Interrupted);

    let after = projected(&ledger);
    assert_eq!(&after[..committed.len()], committed.as_slice());
    let appended = &after[committed.len()..];
    assert_eq!(appended[0].kind, "transcript");
    assert_eq!(appended[0].payload["eventId"], "interrupted:p3");
    assert_eq!(appended[0].payload["text"], "3, 4");
    assert_eq!(appended[0].payload["isPartial"], 0);
    assert_eq!(appended[1].kind, "activity");
    assert_eq!(appended[1].payload["activity"]["type"], "terminal");
    assert_eq!(
        visible(&ledger, &turn),
        [
            ("f1".into(), "1, 2".into(), false),
            ("t1".into(), "   ".into(), true),
            ("interrupted:p3".into(), "3, 4".into(), false)
        ]
    );
    assert!(
        transcript(&ledger)
            .iter()
            .any(|event| event.turn_id == steer)
    );
}

#[test]
fn a_completed_turn_leaves_unsuperseded_partials_for_a_final_that_may_still_arrive() {
    let (_directory, ledger) = conversation();
    let turn = prompt(&ledger, "prompt");
    stream(&ledger, &turn, &[("p1", ANSWER, "late", true)]);

    settle(&ledger, &turn, DurableTurnPhase::Completed);

    assert_eq!(
        visible(&ledger, &turn),
        [("p1".into(), "late".into(), true)]
    );
}

#[test]
fn conversation_recency_advances_with_its_projected_activity() {
    let (_directory, ledger) = conversation();
    let created = ledger
        .read_agent_chat_summary(CONVERSATION)
        .unwrap()
        .updated_at_unix_ms;
    assert!(created > 1_700_000_000_000);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let turn = prompt(&ledger, "prompt");
    stream(&ledger, &turn, &[("f1", ANSWER, "hi", false)]);
    assert!(
        ledger
            .read_agent_chat_summary(CONVERSATION)
            .unwrap()
            .updated_at_unix_ms
            > created
    );
}

#[test]
fn upgrading_a_v22_ledger_rewrites_no_committed_fact_and_adds_recency() {
    let (directory, ledger) = conversation();
    let turn = prompt(&ledger, "prompt");
    stream(
        &ledger,
        &turn,
        &[
            ("p1", ANSWER, "a", true),
            ("f1", ANSWER, "a", false),
            ("p2", ANSWER, "b", true),
        ],
    );
    let committed = projected(&ledger);
    drop(ledger);
    let path = directory.path().join("gent.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TRIGGER agent_chat_conversation_created_recency; \
             DROP TRIGGER agent_chat_conversation_projection_recency; \
             DROP INDEX agent_chat_transcript_events_by_turn_kind; \
             ALTER TABLE agent_chat_conversations DROP COLUMN updated_at_unix_ms; \
             UPDATE turns SET phase = 'interrupted'; \
             UPDATE gent_schema SET identity = 'gent-fresh-schema-v22' WHERE singleton = 1;",
        )
        .unwrap();

    let ledger = SqliteLedger::open(&path).unwrap();

    assert_eq!(projected(&ledger), committed);
    assert_eq!(
        visible(&ledger, &turn),
        [
            ("f1".into(), "a".into(), false),
            ("p2".into(), "b".into(), true)
        ]
    );
    let after = prompt(&ledger, "after");
    stream(&ledger, &after, &[("f2", ANSWER, "c", false)]);
    assert!(
        ledger
            .read_agent_chat_summary(CONVERSATION)
            .unwrap()
            .updated_at_unix_ms
            > 0
    );
}
