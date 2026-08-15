use gent_ports::{ConversationLedger, Ledger, LedgerError, RunRecord, TurnPhaseUpdate};
use gent_store::SqliteLedger;
use gent_types::{ConversationRecord, DurableTurnPhase, TurnRecord};

fn root() -> RunRecord {
    RunRecord {
        run_id: "run-root".into(),
        parent_run_id: None,
        provider: "claude".into(),
    }
}

fn turn(sequence: u64) -> TurnRecord {
    TurnRecord {
        turn_id: format!("turn-{sequence}"),
        conversation_id: "conversation-a".into(),
        run_id: "run-root".into(),
        sequence,
        phase: DurableTurnPhase::Active,
    }
}

#[test]
fn conversation_run_turn_hierarchy_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let conversation = ConversationRecord {
        conversation_id: "conversation-a".into(),
    };
    ledger
        .create_conversation_run(&conversation, &root())
        .unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-codex".into(),
            parent_run_id: Some("run-root".into()),
            provider: "codex".into(),
        })
        .unwrap();
    ledger.create_turn(&turn(1)).unwrap();
    assert!(matches!(
        ledger
            .replace_turn_phase(
                "turn-1",
                DurableTurnPhase::Active,
                DurableTurnPhase::WaitingQuestion
            )
            .unwrap(),
        TurnPhaseUpdate::Applied(TurnRecord {
            phase: DurableTurnPhase::WaitingQuestion,
            ..
        })
    ));
    drop(ledger);

    let restarted = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        restarted.find_conversation("conversation-a").unwrap(),
        Some(conversation)
    );
    assert_eq!(
        restarted
            .list_conversation_runs("conversation-a")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        restarted.find_turn("turn-1").unwrap().unwrap().phase,
        DurableTurnPhase::WaitingQuestion
    );
}

#[test]
fn hierarchy_is_immutable_and_turn_sequences_are_unique() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = ConversationRecord {
        conversation_id: "conversation-a".into(),
    };
    ledger
        .create_conversation_run(&conversation, &root())
        .unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "child".into(),
            parent_run_id: Some("run-root".into()),
            provider: "codex".into(),
        })
        .unwrap();
    assert_eq!(
        ledger
            .list_conversation_runs("conversation-a")
            .unwrap()
            .len(),
        2
    );
    ledger.create_turn(&turn(1)).unwrap();
    assert!(ledger.create_turn(&turn(1)).is_err());
    assert!(matches!(
        ledger.create_turn(&TurnRecord {
            conversation_id: "other".into(),
            ..turn(2)
        }),
        Err(LedgerError::Invariant(_))
    ));
}

#[test]
fn conversation_index_counts_runs_and_orders_newest_first() {
    let ledger = SqliteLedger::in_memory().unwrap();
    for (conversation_id, run_id) in [("older", "run-older"), ("newer", "run-newer")] {
        ledger
            .create_conversation_run(
                &ConversationRecord {
                    conversation_id: conversation_id.into(),
                },
                &RunRecord {
                    run_id: run_id.into(),
                    parent_run_id: None,
                    provider: "claude".into(),
                },
            )
            .unwrap();
    }
    ledger
        .create_run(&RunRecord {
            run_id: "run-newer-child".into(),
            parent_run_id: Some("run-newer".into()),
            provider: "codex".into(),
        })
        .unwrap();
    assert_eq!(
        ledger.list_conversations().unwrap(),
        vec![
            gent_types::ConversationListItem {
                conversation_id: "newer".into(),
                run_count: 2,
            },
            gent_types::ConversationListItem {
                conversation_id: "older".into(),
                run_count: 1,
            },
        ]
    );
}
