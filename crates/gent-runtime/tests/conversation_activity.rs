use gent_ports::{ConversationLedger, Ledger, RunRecord};
use gent_runtime::{
    ConversationActivityAuthority, ConversationActivityRead, ConversationActivityResult,
    ConversationActivityService,
};
use gent_store::SqliteLedger;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, ConversationRecord,
    HostEpoch, WorkPhase,
};

fn scope(cursor: u64) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: "turn-1".into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

fn start(cursor: u64) -> ConversationActivityFact {
    ConversationActivityFact::TurnStarted {
        scope: scope(cursor),
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
        service.record(&start(1)).unwrap(),
        ConversationActivityResult::DeniedObserver
    );
    assert_eq!(
        service.resume("conversation-1", "run-1", 0).unwrap(),
        ConversationActivityResult::DeniedObserver
    );
}

#[test]
fn approved_service_persists_reduced_facts_and_cursor_resume() {
    let ledger = SqliteLedger::in_memory().unwrap();
    setup(&ledger);
    let service =
        ConversationActivityService::new(ledger.clone(), ConversationActivityAuthority::Approved);
    let ConversationActivityResult::Applied(first) = service.record(&start(1)).unwrap() else {
        panic!("expected activity")
    };
    assert_eq!(first.cursor, 1);
    let work = ConversationActivityFact::WorkPhase {
        scope: scope(2),
        work_id: "command-1".into(),
        kind: ActivityWorkKind::Command,
        phase: WorkPhase::Running,
    };
    let ConversationActivityResult::Applied(second) = service.record(&work).unwrap() else {
        panic!("expected activity")
    };
    assert_eq!(second.cursor, 2);
    assert_eq!(second.work.len(), 1);
    let ConversationActivityResult::Resumed(records) =
        service.resume("conversation-1", "run-1", 0).unwrap()
    else {
        panic!("expected resume")
    };
    assert_eq!(
        records
            .into_iter()
            .map(|activity| activity.cursor)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn activity_facts_obey_the_host_fence_before_reduction() {
    let ledger = SqliteLedger::in_memory().unwrap();
    setup(&ledger);
    ledger.close_ingress(HostEpoch(1)).unwrap();
    let service = ConversationActivityService::new(ledger, ConversationActivityAuthority::Approved);
    assert!(service.record(&start(1)).is_err());
}

#[test]
fn activity_read_uses_snapshot_when_a_bounded_delta_cannot_be_complete() {
    let ledger = SqliteLedger::in_memory().unwrap();
    setup(&ledger);
    let service = ConversationActivityService::new(ledger, ConversationActivityAuthority::Approved);
    service.record(&start(1)).unwrap();
    for cursor in 2..=129 {
        let fact = ConversationActivityFact::WorkPhase {
            scope: scope(cursor),
            work_id: "command-1".into(),
            kind: ActivityWorkKind::Command,
            phase: WorkPhase::Running,
        };
        service.record(&fact).unwrap();
    }
    assert!(matches!(
        service.read("conversation-1", "run-1", 0).unwrap(),
        ConversationActivityRead::Snapshot(activity) if activity.cursor == 129
    ));
}
