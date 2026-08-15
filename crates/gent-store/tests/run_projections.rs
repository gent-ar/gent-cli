use gent_ports::{Ledger, LedgerError, RunProjectionLedger, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, RunLifecycleProjection, RunProjectionRecord, TurnPhase};

fn record(cursor: u64) -> RunProjectionRecord {
    RunProjectionRecord {
        run_id: "run-a".into(),
        host_epoch: HostEpoch(1),
        projection: RunLifecycleProjection {
            cursor,
            root_phase: TurnPhase::Processing,
            ..RunLifecycleProjection::default()
        },
    }
}

#[test]
fn projection_survives_restart_and_rejects_regression_or_conflicts() {
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
    let latest = record(4);
    ledger.save_run_projection(&latest).unwrap();
    ledger.save_run_projection(&latest).unwrap();
    assert!(matches!(
        ledger.save_run_projection(&record(3)),
        Err(LedgerError::Invariant(_))
    ));
    let mut conflict = latest.clone();
    conflict.projection.has_error = true;
    assert!(matches!(
        ledger.save_run_projection(&conflict),
        Err(LedgerError::Invariant(_))
    ));
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted.find_run_projection("run-a").unwrap(),
        Some(latest)
    );
}

#[test]
fn projection_requires_an_existing_run() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(matches!(
        ledger.save_run_projection(&record(1)),
        Err(LedgerError::Invariant(_))
    ));
}
