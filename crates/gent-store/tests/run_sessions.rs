use gent_ports::{Ledger, LedgerError, RunRecord, RunSessionBinding};
use gent_store::SqliteLedger;

fn run() -> RunRecord {
    RunRecord {
        run_id: "run-a".into(),
        parent_run_id: None,
        provider: "claude".into(),
    }
}

#[test]
fn session_binding_is_durable_and_immutable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.create_run(&run()).unwrap();
    let binding = RunSessionBinding {
        run_id: "run-a".into(),
        provider_session_id: "provider-session-a".into(),
    };
    ledger.save_run_session_binding(&binding).unwrap();
    ledger.save_run_session_binding(&binding).unwrap();
    assert!(matches!(
        ledger.save_run_session_binding(&RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "provider-session-b".into(),
        }),
        Err(LedgerError::Invariant(_))
    ));
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted.find_run_session_binding("run-a").unwrap(),
        Some(binding)
    );
}
