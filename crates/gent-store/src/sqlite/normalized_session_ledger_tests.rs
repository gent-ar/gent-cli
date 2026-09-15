use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, Ledger, NormalizedSessionBatchLedger, RunLifecycleFactLedger,
    TranscriptLedger,
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
            attachment_ids: vec![],
            tool_source_ids: vec![],
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
    assert_eq!(first.transcript_cursor, Some(2));
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
        2
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
    let transcript = ledger
        .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 10)
        .unwrap();
    assert_eq!(transcript.events.len(), 1);
    assert_ne!(transcript.events[0].event_id, "transcript-a");
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

#[test]
fn a_repeated_tool_phase_is_recorded_once_and_a_new_phase_still_advances() {
    let (ledger, turn_id) = prepared();
    let tool = |index: usize, phase: gent_types::ToolPhase| NormalizedSessionBatch {
        lifecycle_event_id: format!("lifecycle-tool-{index}"),
        transcript: None,
        activity_event_id: Some(format!("activity-tool-{index}")),
        activity: Some(ConversationActivityFact::ToolActivity {
            scope: ConversationActivityScope {
                conversation_id: "conversation-a".into(),
                run_id: "run-a".into(),
                turn_id: turn_id.clone(),
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
            activity: gent_types::ToolActivity {
                tool_use_id: "tool-1".into(),
                tool_name: "Bash".into(),
                phase,
                output_digest: None,
                parent_tool_use_id: None,
            },
        }),
        ..batch(turn_id.clone())
    };
    let recorded = [
        gent_types::ToolPhase::Started,
        gent_types::ToolPhase::Started,
        gent_types::ToolPhase::Started,
        gent_types::ToolPhase::Completed,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, phase)| {
        ledger
            .append_normalized_session_batch(&tool(index, phase))
            .unwrap()
            .activity_cursor
    })
    .collect::<Vec<_>>();

    assert_eq!(&recorded[1..3], &[recorded[0], recorded[0]]);
    assert!(recorded[3] > recorded[0]);
    let phases = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 20)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. } => Some(activity.phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            gent_types::ToolPhase::Started,
            gent_types::ToolPhase::Completed
        ]
    );
    assert_eq!(
        ledger
            .append_normalized_session_batch(&tool(1, gent_types::ToolPhase::Started))
            .unwrap()
            .activity_cursor,
        recorded[0]
    );
}

#[test]
fn a_provider_interruption_appends_the_unfinished_reply_before_its_terminal_fact() {
    let (ledger, turn_id) = prepared();
    let scope = ConversationActivityScope {
        conversation_id: "conversation-a".into(),
        run_id: "run-a".into(),
        turn_id: turn_id.clone(),
        host_epoch: HostEpoch(1),
        cursor: 0,
    };
    let partial = |index: usize, text: &str| NormalizedSessionBatch {
        lifecycle_event_id: format!("lifecycle-partial-{index}"),
        transcript: Some(NormalizedTranscriptAppend {
            event_id: format!("partial-{index}"),
            turn_id: turn_id.clone(),
            run_id: "run-a".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: text.into(),
            is_partial: true,
        }),
        activity_event_id: None,
        activity: None,
        ..batch(turn_id.clone())
    };
    ledger
        .append_normalized_session_batch(&partial(1, "Half "))
        .unwrap();
    ledger
        .append_normalized_session_batch(&partial(2, "done"))
        .unwrap();
    let before = ledger
        .agent_chat_projection_page(&AgentChatConversationId("conversation-a".into()), 0, 100)
        .unwrap()
        .events;

    ledger
        .append_normalized_session_batch(&NormalizedSessionBatch {
            lifecycle_event_id: "lifecycle-terminal".into(),
            transcript: None,
            activity_event_id: Some("activity-terminal".into()),
            activity: Some(ConversationActivityFact::Terminal {
                scope,
                phase: gent_types::TurnPhase::Interrupted,
                cause: None,
            }),
            ..batch(turn_id.clone())
        })
        .unwrap();

    let after = ledger
        .agent_chat_projection_page(&AgentChatConversationId("conversation-a".into()), 0, 100)
        .unwrap()
        .events;
    assert_eq!(&after[..before.len()], before.as_slice());
    let appended = after[before.len()..]
        .iter()
        .filter(|event| event.kind != "lifecycle")
        .map(|event| {
            (
                event.kind.as_str(),
                event.payload["eventId"].as_str().unwrap_or_default(),
                event.payload["text"].as_str().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        appended,
        [
            ("transcript", "interrupted:partial-2", "Half done"),
            ("activity", "", "")
        ]
    );
}
