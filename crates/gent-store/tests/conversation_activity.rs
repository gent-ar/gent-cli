use gent_ports::{ConversationActivityLedger, ConversationLedger, LedgerError, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{
    CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivity, ConversationActivityRecord,
    ConversationActivityState, ConversationRecord, HostEpoch, RootActivity, TurnPhase,
};

fn record(cursor: u64) -> ConversationActivityRecord {
    ConversationActivityRecord {
        activity: ConversationActivity {
            schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            host_epoch: HostEpoch(1),
            revision: cursor,
            activity_sequence: cursor,
            cursor,
            active_turn_id: Some("turn".into()),
            root_phase: TurnPhase::Processing,
            state: ConversationActivityState::Thinking,
            pending_decision_id: None,
            work: Vec::new(),
            has_error: false,
        },
        root_activity: RootActivity::Generating,
        terminal_turn_ids: Vec::new(),
        transition: None,
    }
}

fn create_conversation(ledger: &SqliteLedger) {
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation".into(),
            },
            &RunRecord {
                run_id: "run".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
}

#[test]
fn activity_projection_survives_restart_at_its_latest_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    create_conversation(&ledger);
    ledger.save_conversation_activity(&record(1)).unwrap();
    let latest = record(2);
    ledger.save_conversation_activity(&latest).unwrap();
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted
            .find_conversation_activity("conversation", "run")
            .unwrap(),
        Some(latest)
    );
}

#[test]
fn activity_projection_retries_are_idempotent_and_ordered() {
    let ledger = SqliteLedger::in_memory().unwrap();
    create_conversation(&ledger);
    let latest = record(2);
    ledger.save_conversation_activity(&latest).unwrap();
    ledger.save_conversation_activity(&latest).unwrap();
    assert!(matches!(
        ledger.save_conversation_activity(&record(1)),
        Err(LedgerError::Invariant(_))
    ));
    let mut conflict = latest;
    conflict.activity.has_error = true;
    assert!(matches!(
        ledger.save_conversation_activity(&conflict),
        Err(LedgerError::Invariant(_))
    ));
}

#[test]
fn activity_projection_requires_matching_durable_lineage() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(matches!(
        ledger.save_conversation_activity(&record(1)),
        Err(LedgerError::Invariant(_))
    ));
    create_conversation(&ledger);
    let mut mismatched = record(1);
    mismatched.activity.conversation_id = "other".into();
    assert!(matches!(
        ledger.save_conversation_activity(&mismatched),
        Err(LedgerError::Invariant(_))
    ));
}
