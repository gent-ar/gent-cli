use gent_core::Run;
use gent_ports::{Ledger, RunCheckpointLedger, RunLifecycleFactLedger, TurnPhaseUpdate};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
    ConversationRecord, DurableTurnPhase, Event, HostEpoch, NormalizedProviderEvent,
    NormalizedSessionLifecycle, ReceiptId, RunCheckpointRecord, RunLifecycleFact, TurnRecord,
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
            kind: "normalizedSessionLifecycle".into(),
            payload: serde_json::json!({ "runId": "run-root", "lifecycle": lifecycle }),
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
    let status = coordinator.conversation_status("conversation-a").unwrap();
    assert_eq!(status.runs.len(), 1);
    assert_eq!(status.runs[0].active_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
        status.runs[0].live_status.as_ref().unwrap().host_epoch,
        HostEpoch(3)
    );
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
