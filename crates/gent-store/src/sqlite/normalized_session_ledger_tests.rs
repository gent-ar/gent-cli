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
            ConversationActivityFact::TurnStarted { scope, .. } => scope.host_epoch = HostEpoch(2),
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
                input: None,
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
fn tool_arguments_arriving_after_started_are_durable_and_replayed() {
    let (ledger, turn_id) = prepared();
    let tool = |index, input| NormalizedSessionBatch {
        lifecycle_event_id: format!("tool-input-lifecycle-{index}"),
        transcript: None,
        activity_event_id: Some(format!("tool-input-activity-{index}")),
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
                phase: gent_types::ToolPhase::Started,
                input,
                output_digest: None,
                parent_tool_use_id: None,
            },
        }),
        ..batch(turn_id.clone())
    };
    let empty = ledger
        .append_normalized_session_batch(&tool(0, None))
        .unwrap()
        .activity_cursor;
    let input = Some(serde_json::json!({"command":"cargo test"}));
    let full = ledger
        .append_normalized_session_batch(&tool(1, input.clone()))
        .unwrap()
        .activity_cursor;
    assert!(full > empty);
    for (index, repeated) in [(2, input.clone()), (3, None)] {
        assert_eq!(
            ledger
                .append_normalized_session_batch(&tool(index, repeated))
                .unwrap()
                .activity_cursor,
            full
        );
    }
    let facts = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 20)
        .unwrap()
        .facts;
    let inputs = facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. } => Some(activity.input),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(inputs, [None, input]);
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

fn output(turn_id: &str, index: usize) -> NormalizedSessionBatch {
    NormalizedSessionBatch {
        lifecycle_event_id: format!("lifecycle-{index}"),
        lifecycle: NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::Output {
                text: format!("token-{index}"),
                is_partial: true,
            },
        },
        transcript: Some(NormalizedTranscriptAppend {
            event_id: format!("transcript-{index}"),
            turn_id: turn_id.into(),
            run_id: "run-a".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: format!("token-{index}"),
            is_partial: true,
        }),
        activity_event_id: None,
        activity: None,
        ..batch(turn_id.into())
    }
}

#[test]
fn a_drained_batch_of_facts_is_recorded_in_order_or_not_at_all() {
    let (ledger, turn_id) = prepared();
    let drained = (0..3)
        .map(|index| output(&turn_id, index))
        .collect::<Vec<_>>();
    let mut colliding = drained.clone();
    colliding[2].transcript.as_mut().unwrap().event_id = "transcript-0".into();
    assert!(
        ledger
            .append_normalized_session_batches(&colliding)
            .is_err()
    );
    assert!(ledger.read_event_page(0, 100).unwrap().events.is_empty());
    let recorded = ledger.append_normalized_session_batches(&drained).unwrap();
    let cursors = recorded
        .iter()
        .map(|result| result.lifecycle_cursor)
        .collect::<Vec<_>>();
    assert_eq!(cursors, [1, 2, 3]);
    assert_eq!(
        ledger.append_normalized_session_batches(&drained).unwrap(),
        recorded
    );
}

fn activity_batch(
    turn_id: &str,
    index: u32,
    fact: ConversationActivityFact,
) -> NormalizedSessionBatch {
    NormalizedSessionBatch {
        lifecycle_event_id: format!("lifecycle-late-{index}"),
        transcript: None,
        activity_event_id: Some(format!("activity-late-{index}")),
        activity: Some(fact),
        ..batch(turn_id.into())
    }
}

fn exec(turn_id: &str, tool_name: &str, phase: gent_types::ToolPhase) -> ConversationActivityFact {
    ConversationActivityFact::ToolActivity {
        scope: scope(turn_id),
        activity: gent_types::ToolActivity {
            tool_use_id: "exec-1".into(),
            tool_name: tool_name.into(),
            phase,
            output_digest: None,
            input: None,
            parent_tool_use_id: None,
        },
    }
}

fn scope(turn_id: &str) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation-a".into(),
        run_id: "run-a".into(),
        turn_id: turn_id.into(),
        host_epoch: HostEpoch(1),
        cursor: 0,
    }
}

fn pending_permission(
    turn_id: &str,
    child_id: Option<&str>,
) -> gent_types::PermissionDecisionRequest {
    gent_types::PermissionDecisionRequest {
        binding: gent_types::PermissionDecisionBinding {
            decision_id: gent_types::AgentChatDecisionId("0".into()),
            request_idempotency_key: "codex:run-a:0".into(),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
            turn_id: turn_id.into(),
            policy_id: "policy-1".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: gent_types::PermissionRequestDigest("a".repeat(64)),
        },
        request: gent_types::PermissionRequest {
            tool_name: "Edit".into(),
            category: gent_types::PermissionCategory::Edit,
            input: None,
            child_id: child_id.map(str::to_owned),
        },
    }
}

#[test]
fn an_interrupted_terminal_is_the_last_tool_and_decision_fact_of_its_turn() {
    use gent_ports::PendingPermissionLedger;
    use gent_types::{ToolPhase, TurnPhase};
    let (ledger, turn_id) = prepared();
    let started = exec(&turn_id, "commandExecution", ToolPhase::Started);
    ledger
        .append_normalized_session_batches(&[activity_batch(&turn_id, 0, started.clone())])
        .unwrap();
    ledger
        .save_pending_permission(&pending_permission(&turn_id, None))
        .unwrap();
    let terminal = ConversationActivityFact::Terminal {
        scope: scope(&turn_id),
        phase: TurnPhase::Interrupted,
        cause: None,
    };
    let late = [
        activity_batch(&turn_id, 1, terminal),
        activity_batch(&turn_id, 2, started),
        activity_batch(&turn_id, 3, exec(&turn_id, "fileChange", ToolPhase::Failed)),
    ];
    let recorded = ledger.append_normalized_session_batches(&late).unwrap();
    assert_eq!(recorded[1].activity_cursor, recorded[0].activity_cursor);
    assert_eq!(recorded[2].activity_cursor, recorded[0].activity_cursor);
    let facts = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. } => {
                format!("{}:{:?}", activity.tool_name, activity.phase)
            }
            other => serde_json::to_value(&other).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned(),
        })
        .filter(|kind| kind.contains(':') || kind.starts_with("decision") || kind == "terminal")
        .collect::<Vec<_>>();
    assert_eq!(
        facts,
        [
            "commandExecution:Started",
            "decisionPending",
            "decisionSettled",
            "commandExecution:Failed",
            "terminal",
        ]
    );
}

#[test]
fn a_background_command_outlives_its_interrupted_turn_until_its_own_end() {
    use gent_ports::NormalizedSessionBatchLedger;
    use gent_types::{ToolPhase, TurnPhase};
    let (ledger, turn_id) = prepared();
    let launch = [
        activity_batch(
            &turn_id,
            0,
            exec(&turn_id, "commandExecution", ToolPhase::Started),
        ),
        activity_batch(
            &turn_id,
            1,
            exec(&turn_id, "commandExecution", ToolPhase::Background),
        ),
        activity_batch(&turn_id, 2, terminal(&turn_id, TurnPhase::Interrupted)),
    ];
    let launched = ledger.append_normalized_session_batches(&launch).unwrap();
    let later = ledger
        .start_provider_turn("conversation-a", "run-a", HostEpoch(1))
        .unwrap();
    let ends = [
        activity_batch(
            &later,
            3,
            exec(&later, "commandExecution", ToolPhase::Interrupted),
        ),
        activity_batch(
            &turn_id,
            4,
            exec(&turn_id, "commandExecution", ToolPhase::Started),
        ),
    ];
    let recorded = ledger.append_normalized_session_batches(&ends).unwrap();
    assert_ne!(recorded[0].activity_cursor, launched[2].activity_cursor);
    assert_eq!(recorded[1].activity_cursor, launched[2].activity_cursor);
    let facts = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 50)
        .unwrap()
        .facts;
    let tools = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { scope, activity } => {
                Some((scope.turn_id.as_str(), activity.phase.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tools,
        [
            (turn_id.as_str(), ToolPhase::Started),
            (turn_id.as_str(), ToolPhase::Background),
            (turn_id.as_str(), ToolPhase::Interrupted),
        ]
    );
}

fn child_started(turn_id: &str) -> ConversationActivityFact {
    ConversationActivityFact::SubagentStarted {
        scope: scope(turn_id),
        child_id: "child-1".into(),
        parent_tool_use_id: "spawn-1".into(),
    }
}

fn child_phase(turn_id: &str, phase: gent_types::WorkPhase) -> ConversationActivityFact {
    ConversationActivityFact::WorkPhase {
        scope: scope(turn_id),
        work_id: "child-1".into(),
        kind: gent_types::ActivityWorkKind::Subagent,
        phase,
    }
}

fn terminal(turn_id: &str, phase: gent_types::TurnPhase) -> ConversationActivityFact {
    ConversationActivityFact::Terminal {
        scope: scope(turn_id),
        phase,
        cause: None,
    }
}

fn kinds(ledger: &SqliteLedger) -> Vec<String> {
    ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .map(|fact| {
            let value = serde_json::to_value(&fact).unwrap();
            match value["phase"].as_str() {
                Some(phase) => format!("{}:{phase}", value["type"].as_str().unwrap()),
                None => value["type"].as_str().unwrap().to_owned(),
            }
        })
        .filter(|kind| {
            ["subagentStarted", "decision", "workPhase", "terminal"]
                .iter()
                .any(|prefix| kind.starts_with(prefix))
        })
        .collect()
}

#[test]
fn a_ready_root_terminal_leaves_a_running_childs_pending_decision_pending() {
    use gent_ports::PendingPermissionLedger;
    let (ledger, turn_id) = prepared();
    ledger
        .append_normalized_session_batches(&[activity_batch(&turn_id, 0, child_started(&turn_id))])
        .unwrap();
    let request = pending_permission(&turn_id, Some("child-1"));
    ledger.save_pending_permission(&request).unwrap();
    ledger
        .append_normalized_session_batches(&[activity_batch(
            &turn_id,
            1,
            terminal(&turn_id, gent_types::TurnPhase::Ready),
        )])
        .unwrap();
    assert_eq!(
        kinds(&ledger),
        ["subagentStarted", "decisionPending", "terminal:ready"]
    );
    assert_eq!(
        ledger
            .pending_permission(&request.binding.conversation_id, &request.binding.run_id)
            .unwrap(),
        Some(request)
    );
}

#[test]
fn an_interrupted_root_terminal_ends_its_running_child_and_nothing_reopens_it() {
    use gent_ports::PendingPermissionLedger;
    use gent_types::WorkPhase;
    let (ledger, turn_id) = prepared();
    ledger
        .append_normalized_session_batches(&[activity_batch(&turn_id, 0, child_started(&turn_id))])
        .unwrap();
    ledger
        .save_pending_permission(&pending_permission(&turn_id, Some("child-1")))
        .unwrap();
    let recorded = ledger
        .append_normalized_session_batches(&[
            activity_batch(
                &turn_id,
                1,
                terminal(&turn_id, gent_types::TurnPhase::Interrupted),
            ),
            activity_batch(&turn_id, 2, child_phase(&turn_id, WorkPhase::Running)),
        ])
        .unwrap();
    assert_eq!(
        kinds(&ledger),
        [
            "subagentStarted",
            "decisionPending",
            "decisionSettled",
            "workPhase:interrupted",
            "terminal:interrupted",
        ]
    );
    assert!(recorded[1].activity_cursor < recorded[0].activity_cursor);
}
