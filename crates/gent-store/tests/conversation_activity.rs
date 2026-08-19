use gent_ports::{ConversationActivityLedger, ConversationLedger, LedgerError, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, ConversationRecord, HostEpoch,
};

fn fact(cursor: u64) -> ConversationActivityFact {
    ConversationActivityFact::TurnStarted {
        scope: ConversationActivityScope {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            turn_id: format!("turn-{cursor}"),
            host_epoch: HostEpoch(1),
            cursor,
        },
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
fn activity_facts_survive_restart_in_cursor_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    create_conversation(&ledger);
    ledger.append_conversation_activity(&fact(1)).unwrap();
    ledger.append_conversation_activity(&fact(2)).unwrap();
    drop(ledger);
    let page = SqliteLedger::open(path)
        .unwrap()
        .read_conversation_activity_page("conversation", "run", 0, 64)
        .unwrap();
    assert_eq!(page.facts, vec![fact(1), fact(2)]);
    assert_eq!(page.next_after_cursor, None);
}

#[test]
fn activity_facts_are_idempotent_and_immutable() {
    let ledger = SqliteLedger::in_memory().unwrap();
    create_conversation(&ledger);
    ledger.append_conversation_activity(&fact(2)).unwrap();
    ledger.append_conversation_activity(&fact(2)).unwrap();
    let mut conflicting = fact(2);
    if let ConversationActivityFact::TurnStarted { scope } = &mut conflicting {
        scope.turn_id = "other".into();
    }
    assert!(matches!(
        ledger.append_conversation_activity(&conflicting),
        Err(LedgerError::Invariant(_))
    ));
}

#[test]
fn activity_page_is_bounded_and_names_its_next_cursor() {
    let ledger = SqliteLedger::in_memory().unwrap();
    create_conversation(&ledger);
    for cursor in 1..=3 {
        ledger.append_conversation_activity(&fact(cursor)).unwrap();
    }
    let page = ledger
        .read_conversation_activity_page("conversation", "run", 0, 2)
        .unwrap();
    assert_eq!(page.facts, vec![fact(1), fact(2)]);
    assert_eq!(page.next_after_cursor, Some(2));
    assert_eq!(
        ledger
            .read_conversation_activity_page("conversation", "run", 2, 2)
            .unwrap()
            .facts,
        vec![fact(3)]
    );
}
