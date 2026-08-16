use gent_ports::{Ledger, RunLease, RunLeaseClaim, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, RunVersionLock};

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/locked/claude".into(),
        file_identity: "identity".into(),
        digest_sha256: "a".repeat(64),
        version: "1".into(),
        compatibility_entry: "entry".into(),
    }
}

fn lease(owner: &str) -> RunLease {
    RunLease {
        run_id: "run-chat".into(),
        coordinator_id: owner.into(),
        host_epoch: HostEpoch(1),
    }
}

#[test]
fn activation_locks_and_leases_an_existing_run_without_mutating_its_lineage() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-chat".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    assert_eq!(
        ledger
            .activate_existing_run_start(&lock(), &lease("daemon-a"))
            .unwrap(),
        RunLeaseClaim::Acquired(lease("daemon-a"))
    );
    assert_eq!(
        ledger.find_run("run-chat").unwrap().unwrap().parent_run_id,
        None
    );
    assert_eq!(
        ledger.find_run_version_lock("run-chat").unwrap(),
        Some(lock())
    );
    assert_eq!(
        ledger.find_run_lease("run-chat").unwrap(),
        Some(lease("daemon-a"))
    );
}

#[test]
fn activation_is_retry_safe_but_rejects_lock_replacement_or_another_owner() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-chat".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    ledger
        .activate_existing_run_start(&lock(), &lease("daemon-a"))
        .unwrap();
    assert_eq!(
        ledger
            .activate_existing_run_start(&lock(), &lease("daemon-a"))
            .unwrap(),
        RunLeaseClaim::Acquired(lease("daemon-a"))
    );
    assert!(matches!(
        ledger
            .activate_existing_run_start(&lock(), &lease("daemon-b"))
            .unwrap(),
        RunLeaseClaim::Contended(_)
    ));
    let mut changed = lock();
    changed.digest_sha256 = "b".repeat(64);
    assert!(
        ledger
            .activate_existing_run_start(&changed, &lease("daemon-a"))
            .is_err()
    );
}
