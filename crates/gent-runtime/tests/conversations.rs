use gent_core::Run;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger,
    RunCheckpointLedger, RunLifecycleFactLedger, TurnPhaseUpdate,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, CapabilitySet, ConversationArtifact,
    ConversationArtifactKind, ConversationArtifactStatus, ConversationRecord, DurableTurnPhase,
    Event, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent,
    NormalizedSessionLifecycle, ReceiptId, RootActivity, RunCheckpointRecord, RunLifecycleFact,
    TurnRecord, WorkspaceRecord,
};

fn coordinator() -> Coordinator<SqliteLedger> {
    Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default())
}

#[test]
fn provider_switch_retains_conversation_and_turns_are_monotonic() {
    let coordinator = coordinator();
    let root = Run {
        id: "run-root".into(),
        parent_run_id: None,
        provider: "claude".into(),
    };
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &root,
        )
        .unwrap();
    coordinator
        .switch_provider(&root, "run-codex".into(), "codex".into())
        .unwrap();
    assert_eq!(
        coordinator
            .conversation_runs("conversation-a")
            .unwrap()
            .len(),
        2
    );
    coordinator
        .create_turn(&TurnRecord {
            turn_id: "turn-1".into(),
            conversation_id: "conversation-a".into(),
            run_id: root.id.clone(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    assert!(matches!(
        coordinator
            .transition_turn(
                "turn-1",
                DurableTurnPhase::Active,
                DurableTurnPhase::Completed
            )
            .unwrap(),
        TurnPhaseUpdate::Applied(TurnRecord {
            phase: DurableTurnPhase::Completed,
            ..
        })
    ));
    assert!(
        coordinator
            .transition_turn(
                "turn-1",
                DurableTurnPhase::Completed,
                DurableTurnPhase::Active
            )
            .is_err()
    );
}

#[test]
fn conversation_index_exposes_only_identity_and_run_count() {
    let coordinator = coordinator();
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-root".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    assert_eq!(
        coordinator.conversations().unwrap(),
        vec![gent_types::ConversationListItem {
            conversation_id: "conversation-a".into(),
            run_count: 1,
        }]
    );
}

#[test]
fn stale_turn_transition_preserves_durable_state() {
    let coordinator = coordinator();
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-root".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    coordinator
        .create_turn(&TurnRecord {
            turn_id: "turn-1".into(),
            conversation_id: "conversation-a".into(),
            run_id: "run-root".into(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    assert!(matches!(
        coordinator
            .transition_turn(
                "turn-1",
                DurableTurnPhase::WaitingQuestion,
                DurableTurnPhase::Active
            )
            .unwrap(),
        TurnPhaseUpdate::Current(TurnRecord {
            phase: DurableTurnPhase::Active,
            ..
        })
    ));
}

#[test]
fn title_and_recap_provenance_is_durable_and_read_only() {
    let coordinator = coordinator();
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-root".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    coordinator
        .create_conversation_artifact(&ConversationArtifact {
            artifact_id: "title-1".into(),
            conversation_id: "conversation-a".into(),
            kind: ConversationArtifactKind::Title,
            source_turn_ids: vec!["turn-1".into()],
            provider: "claude".into(),
            model_version: "1".into(),
            input_digest: "sha256:input".into(),
            status: ConversationArtifactStatus::Completed,
            text: Some("A title".into()),
            supersedes_artifact_id: None,
        })
        .unwrap();
    assert_eq!(
        coordinator
            .conversation_artifacts("conversation-a")
            .unwrap()[0]
            .text,
        Some("A title".into())
    );
}

#[test]
fn status_never_exposes_provider_sessions_and_keeps_projection_identity() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-root".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    let lifecycle = NormalizedSessionLifecycle::Event {
        event: NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-1".into(),
        },
    };
    let source = ledger
        .append_event(&Event {
            cursor: 0,
            event_id: "status-source".into(),
            receipt_id: ReceiptId("status-receipt".into()),
            host_epoch: HostEpoch(3),
            kind: "providerLifecycle".into(),
            payload: serde_json::json!({
                "runId": "run-root",
                "event": NormalizedProviderEvent::TurnStarted {
                    turn_id: "turn-1".into(),
                }
            }),
        })
        .unwrap();
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: "run-root".into(),
            event_id: "status-source".into(),
            host_epoch: HostEpoch(3),
            cursor: source.cursor,
            lifecycle,
        })
        .unwrap();
    coordinator
        .create_turn(&TurnRecord {
            turn_id: "turn-1".into(),
            conversation_id: "conversation-a".into(),
            run_id: "run-root".into(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    let status = coordinator.conversation_status("conversation-a").unwrap();
    assert_eq!(status.runs.len(), 1);
    assert_eq!(status.runs[0].active_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
        status.runs[0].live_status.as_ref().unwrap().host_epoch,
        HostEpoch(3)
    );
}

const DISPATCH_EPOCH: HostEpoch = HostEpoch(1);
const DISPATCH_COORDINATOR: &str = "gentd-1";

fn dispatched_root_turn(ledger: &SqliteLedger) -> gent_types::AgentChatPromptSaved {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: DISPATCH_EPOCH,
                conversation_id: AgentChatConversationId("conversation-a".into()),
                run_id: AgentChatRunId("run-root".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let root = ledger
        .save_agent_chat_prompt_with_origin(
            &AgentChatPromptCreate {
                request_id: AgentChatRequestId("root".into()),
                receipt_id: ReceiptId("root".into()),
                host_epoch: DISPATCH_EPOCH,
                conversation_id: AgentChatConversationId("conversation-a".into()),
                disposition: AgentChatPromptDisposition::Send,
                text: "Go".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
            &AgentChatPromptOrigin::User {
                client_request_id: None,
            },
        )
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(
            &root.message.message_id,
            &root.run_id,
            DISPATCH_EPOCH,
        )
        .unwrap();
    ledger
        .claim_agent_chat_prompt_dispatch(
            DISPATCH_COORDINATOR,
            DISPATCH_EPOCH,
            AgentChatProvider::Claude,
        )
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(
            &root.message.message_id,
            DISPATCH_COORDINATOR,
            DISPATCH_EPOCH,
        )
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(
            &root.message.message_id,
            DISPATCH_COORDINATOR,
            DISPATCH_EPOCH,
        )
        .unwrap();
    let signal = NormalizedLifecycleSignal::RootActivity {
        activity: RootActivity::Generating,
    };
    let source = ledger
        .append_event(&Event {
            cursor: 0,
            event_id: "root-activity-source".into(),
            receipt_id: ReceiptId("root-activity-receipt".into()),
            host_epoch: DISPATCH_EPOCH,
            kind: "providerLifecycle".into(),
            payload: serde_json::json!({"runId": root.run_id.0, "signal": signal}),
        })
        .unwrap();
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: root.run_id.0.clone(),
            event_id: "root-activity-source".into(),
            host_epoch: DISPATCH_EPOCH,
            cursor: source.cursor,
            lifecycle: NormalizedSessionLifecycle::Signal { signal },
        })
        .unwrap();
    root
}

#[test]
fn a_dispatched_non_terminal_turn_without_a_normalized_turn_started_event_still_resolves_as_active()
{
    let ledger = SqliteLedger::in_memory().unwrap();
    let root = dispatched_root_turn(&ledger);
    let coordinator = Coordinator::new(ledger, CapabilitySet::default());
    let status = coordinator.conversation_status("conversation-a").unwrap();
    assert_eq!(status.runs.len(), 1);
    assert_eq!(
        status.runs[0].active_turn_id.as_deref(),
        Some(root.message.turn_id.as_str())
    );
}

#[test]
fn a_turn_settled_in_the_ledger_never_resolves_as_active() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let root = dispatched_root_turn(&ledger);
    ledger
        .settle_agent_chat_prompt_terminal(
            &root.message.message_id,
            DISPATCH_COORDINATOR,
            DISPATCH_EPOCH,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    let coordinator = Coordinator::new(ledger, CapabilitySet::default());
    let status = coordinator.conversation_status("conversation-a").unwrap();
    assert_eq!(status.runs.len(), 1);
    assert_eq!(status.runs[0].active_turn_id, None);
    let live = &status.runs[0].live_status.as_ref().unwrap().status;
    assert!(!live.is_processing());
    assert!(!live.has_error());
}

#[test]
fn timeline_preserves_ordered_lifecycle_and_never_serializes_artifact_text() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let root = Run {
        id: "run-root".into(),
        parent_run_id: None,
        provider: "claude".into(),
    };
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &root,
        )
        .unwrap();
    coordinator
        .switch_provider(&root, "run-child".into(), "codex".into())
        .unwrap();
    for (turn_id, run_id, sequence) in [
        ("turn-1", "run-root", 1),
        ("turn-2", "run-root", 2),
        ("turn-3", "run-child", 1),
    ] {
        coordinator
            .create_turn(&TurnRecord {
                turn_id: turn_id.into(),
                conversation_id: "conversation-a".into(),
                run_id: run_id.into(),
                sequence,
                phase: DurableTurnPhase::Completed,
            })
            .unwrap();
    }
    coordinator
        .create_conversation_artifact(&ConversationArtifact {
            artifact_id: "recap-1".into(),
            conversation_id: "conversation-a".into(),
            kind: ConversationArtifactKind::Recap,
            source_turn_ids: vec!["turn-1".into()],
            provider: "claude".into(),
            model_version: "1".into(),
            input_digest: "sha256:prior-input".into(),
            status: ConversationArtifactStatus::Completed,
            text: Some("older private recap text".into()),
            supersedes_artifact_id: None,
        })
        .unwrap();
    coordinator
        .create_conversation_artifact(&ConversationArtifact {
            artifact_id: "recap-2".into(),
            conversation_id: "conversation-a".into(),
            kind: ConversationArtifactKind::Recap,
            source_turn_ids: vec!["turn-1".into(), "turn-2".into()],
            provider: "codex".into(),
            model_version: "1".into(),
            input_digest: "sha256:input".into(),
            status: ConversationArtifactStatus::Completed,
            text: Some("private recap text".into()),
            supersedes_artifact_id: Some("recap-1".into()),
        })
        .unwrap();
    ledger
        .save_run_checkpoint(&RunCheckpointRecord {
            checkpoint_id: "checkpoint-1".into(),
            run_id: "run-root".into(),
            sequence: 1,
            event_cursor: 3,
            state_digest_sha256: "a".repeat(64),
        })
        .unwrap();
    let timeline = coordinator.conversation_timeline("conversation-a").unwrap();
    assert_eq!(timeline.runs.len(), 2);
    assert_eq!(timeline.runs[0].turns.len(), 2);
    assert_eq!(timeline.runs[0].checkpoints.len(), 1);
    assert_eq!(timeline.runs[1].parent_run_id.as_deref(), Some("run-root"));
    assert_eq!(
        timeline.artifacts[1].supersedes_artifact_id.as_deref(),
        Some("recap-1")
    );
    let json = serde_json::to_string(&timeline).unwrap();
    assert!(!json.contains("private recap text"));
    assert!(!json.contains("\"text\""));
}
