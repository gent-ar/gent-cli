use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, RunLifecycleFactLedger, TranscriptLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ConversationActivityFact, ConversationActivityScope,
    HostEpoch, NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

use super::SqliteLedger;

fn prepared() -> (SqliteLedger, String) {
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
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&gent_ports::RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "private-session".into(),
        })
        .unwrap();
    (ledger, prompt.message.turn_id)
}

fn batch(turn_id: String) -> NormalizedSessionBatch {
    NormalizedSessionBatch {
        coordinator_id: "daemon-a".into(),
        conversation_id: "conversation-a".into(),
        run_id: "run-a".into(),
        turn_id: turn_id.clone(),
        host_epoch: HostEpoch(1),
        lifecycle_event_id: "lifecycle-a".into(),
        lifecycle: NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: turn_id.clone(),
            },
        },
        transcript: Some(NormalizedTranscriptAppend {
            event_id: "transcript-a".into(),
            turn_id: turn_id.clone(),
            run_id: "run-a".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "working".into(),
            is_partial: true,
        }),
        activity_event_id: Some("activity-a".into()),
        activity: Some(ConversationActivityFact::TurnStarted {
            scope: ConversationActivityScope {
                conversation_id: "conversation-a".into(),
                run_id: "run-a".into(),
                turn_id,
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
        }),
    }
}

#[test]
fn batch_commits_all_projections_with_exact_retry_cursors() {
    let (ledger, turn_id) = prepared();
    let input = batch(turn_id);
    let first = ledger.append_normalized_session_batch(&input).unwrap();
    let retry = ledger.append_normalized_session_batch(&input).unwrap();
    assert_eq!(first, retry);
    assert_eq!(first.lifecycle_cursor, 1);
    assert_eq!(first.transcript_cursor, Some(1));
    assert_eq!(first.activity_cursor, Some(2));
    let events = ledger.read_event_page(0, 100).unwrap().events;
    assert_eq!(events.len(), 2);
    assert_eq!(
        ledger
            .read_run_lifecycle_fact_page("run-a", 0, 64)
            .unwrap()
            .facts[0]
            .cursor,
        first.lifecycle_cursor
    );
    assert_eq!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
            .unwrap()
            .facts[0]
            .scope()
            .cursor,
        first.activity_cursor.unwrap()
    );
    assert_eq!(
        ledger
            .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 10)
            .unwrap()
            .events
            .len(),
        1
    );
}

#[test]
fn batch_collision_rolls_back_every_prior_projection() {
    let (ledger, turn_id) = prepared();
    let mut input = batch(turn_id);
    input.activity_event_id = Some(input.lifecycle_event_id.clone());
    assert!(ledger.append_normalized_session_batch(&input).is_err());
    assert!(
        ledger
            .find_event(&input.lifecycle_event_id)
            .unwrap()
            .is_none()
    );
    assert!(
        ledger
            .read_run_lifecycle_fact_page("run-a", 0, 64)
            .unwrap()
            .facts
            .is_empty()
    );
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
            .unwrap()
            .facts
            .is_empty()
    );
    assert!(
        ledger
            .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 10)
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn batch_rejects_stale_epoch_and_changed_source_payload() {
    let (ledger, turn_id) = prepared();
    let input = batch(turn_id);
    ledger.append_normalized_session_batch(&input).unwrap();
    let mut changed = input.clone();
    changed.transcript.as_mut().unwrap().text = "changed".into();
    assert!(ledger.append_normalized_session_batch(&changed).is_err());
    let mut stale = input;
    stale.lifecycle_event_id = "lifecycle-stale".into();
    stale.host_epoch = HostEpoch(2);
    if let Some(value) = stale.activity.as_mut() {
        match value {
            ConversationActivityFact::TurnStarted { scope } => scope.host_epoch = HostEpoch(2),
            _ => unreachable!(),
        }
    }
    assert!(ledger.append_normalized_session_batch(&stale).is_err());
}
