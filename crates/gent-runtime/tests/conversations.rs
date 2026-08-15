use gent_core::Run;
use gent_ports::TurnPhaseUpdate;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, ConversationRecord, DurableTurnPhase, TurnRecord};

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
