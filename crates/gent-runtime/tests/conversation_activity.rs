use gent_ports::{ConversationLedger, Ledger, RunRecord};
use gent_runtime::{
    ConversationActivityAuthority, ConversationActivityRead, ConversationActivityResult,
    ConversationActivityService,
};
use gent_store::SqliteLedger;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, ConversationRecord, HostEpoch,
};

fn fact(cursor: u64) -> ConversationActivityFact {
    ConversationActivityFact::TurnStarted {
        scope: ConversationActivityScope {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: format!("turn-{cursor}"),
            host_epoch: HostEpoch(1),
            cursor,
        },
    }
}

fn setup(ledger: &SqliteLedger) {
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-1".into(),
            },
            &RunRecord {
                run_id: "run-1".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
}

#[test]
fn observer_service_does_not_read_or_write_activity() {
    let service = ConversationActivityService::new(
        SqliteLedger::in_memory().unwrap(),
        ConversationActivityAuthority::Observer,
    );
    assert_eq!(
        service.record(&fact(1)).unwrap(),
        ConversationActivityResult::DeniedObserver
    );
    assert_eq!(
        service.read("conversation-1", "run-1", 0).unwrap(),
        ConversationActivityRead::DeniedObserver
    );
}

#[test]
fn approved_service_persists_immutable_facts_and_reads_a_page() {
    let ledger = SqliteLedger::in_memory().unwrap();
    setup(&ledger);
    let service = ConversationActivityService::new(ledger, ConversationActivityAuthority::Approved);
    assert_eq!(
        service.record(&fact(1)).unwrap(),
        ConversationActivityResult::Recorded(fact(1))
    );
    assert_eq!(
        service.record(&fact(2)).unwrap(),
        ConversationActivityResult::Recorded(fact(2))
    );
    assert!(matches!(
        service.read("conversation-1", "run-1", 0).unwrap(),
        ConversationActivityRead::Page(page) if page.facts == vec![fact(1), fact(2)] && page.next_after_cursor.is_none()
    ));
}

#[test]
fn activity_facts_obey_the_host_fence_before_persistence() {
    let ledger = SqliteLedger::in_memory().unwrap();
    setup(&ledger);
    ledger.close_ingress(HostEpoch(1)).unwrap();
    let service = ConversationActivityService::new(ledger, ConversationActivityAuthority::Approved);
    assert!(service.record(&fact(1)).is_err());
}
