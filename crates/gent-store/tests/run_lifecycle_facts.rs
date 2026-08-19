use gent_ports::{Ledger, LedgerError, RunLifecycleFactLedger, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, NormalizedProviderEvent, NormalizedSessionLifecycle, RunLifecycleFact,
};

fn source(ledger: &SqliteLedger, event_id: &str, lifecycle: &NormalizedSessionLifecycle) -> u64 {
    ledger
        .append_event(&gent_types::Event {
            cursor: 0,
            event_id: event_id.into(),
            receipt_id: gent_types::ReceiptId("receipt-a".into()),
            host_epoch: HostEpoch(1),
            kind: "normalizedSessionLifecycle".into(),
            payload: serde_json::json!({ "runId": "run-a", "lifecycle": lifecycle }),
        })
        .unwrap()
        .cursor
}

fn fact(cursor: u64, event_id: &str) -> RunLifecycleFact {
    RunLifecycleFact {
        run_id: "run-a".into(),
        event_id: event_id.into(),
        host_epoch: HostEpoch(1),
        cursor,
        lifecycle: NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            },
        },
    }
}

#[test]
fn facts_survive_restart_and_page_without_retained_projection_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    let template = fact(0, "event-a");
    let cursor = source(&ledger, "event-a", &template.lifecycle);
    let first = fact(cursor, "event-a");
    ledger.append_run_lifecycle_fact(&first).unwrap();
    ledger.append_run_lifecycle_fact(&first).unwrap();
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted
            .read_run_lifecycle_fact_page("run-a", 0, 1)
            .unwrap()
            .facts,
        vec![first]
    );
}

#[test]
fn fact_rejects_a_missing_or_conflicting_source() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    assert!(matches!(
        ledger.append_run_lifecycle_fact(&fact(1, "missing")),
        Err(LedgerError::Invariant(_))
    ));
}
