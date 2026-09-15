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
fn activation_is_retry_safe_and_rejects_another_owner_without_touching_the_lock() {
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
    let mut upgraded = lock();
    upgraded.digest_sha256 = "b".repeat(64);
    assert!(matches!(
        ledger
            .activate_existing_run_start(&upgraded, &lease("daemon-b"))
            .unwrap(),
        RunLeaseClaim::Contended(_)
    ));
    assert_eq!(
        ledger.find_run_version_lock("run-chat").unwrap(),
        Some(lock())
    );
    let mut claurst = lock();
    claurst.provider = "codex".into();
    assert!(
        ledger
            .activate_existing_run_start(&claurst, &lease("daemon-a"))
            .is_err()
    );
}

#[test]
fn activating_a_newly_authorized_executable_rebinds_the_run_and_records_the_previous_lock() {
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
    let mut upgraded = lock();
    upgraded.digest_sha256 = "b".repeat(64);
    upgraded.file_identity = "upgraded".into();
    upgraded.version = "2".into();
    for _ in 0..2 {
        assert_eq!(
            ledger
                .activate_existing_run_start(&upgraded, &lease("daemon-a"))
                .unwrap(),
            RunLeaseClaim::Acquired(lease("daemon-a"))
        );
    }
    assert_eq!(
        ledger.find_run_version_lock("run-chat").unwrap(),
        Some(upgraded.clone())
    );
    let rebinds = ledger
        .read_event_page(0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "runExecutableRebound")
        .collect::<Vec<_>>();
    assert_eq!(rebinds.len(), 1);
    assert_eq!(
        rebinds[0].payload["previous"],
        serde_json::to_value(lock()).unwrap()
    );
    assert_eq!(
        rebinds[0].payload["current"],
        serde_json::to_value(upgraded).unwrap()
    );
}

#[test]
fn retiring_a_provider_session_requires_the_bound_identity_and_is_recorded_once() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-chat".into(),
            parent_run_id: None,
            provider: "codex".into(),
        })
        .unwrap();
    let bound = gent_ports::RunSessionBinding {
        run_id: "run-chat".into(),
        provider_session_id: "thread-a".into(),
    };
    ledger.save_run_session_binding(&bound).unwrap();
    let other = gent_ports::RunSessionBinding {
        provider_session_id: "thread-b".into(),
        ..bound.clone()
    };
    assert!(
        ledger
            .retire_run_session_binding(&other, HostEpoch(1))
            .is_err()
    );
    assert!(
        ledger
            .retire_run_session_binding(&bound, HostEpoch(2))
            .is_err()
    );
    ledger
        .retire_run_session_binding(&bound, HostEpoch(1))
        .unwrap();
    ledger
        .retire_run_session_binding(&bound, HostEpoch(1))
        .unwrap();
    assert!(
        ledger
            .find_run_session_binding("run-chat")
            .unwrap()
            .is_none()
    );
    ledger.save_run_session_binding(&other).unwrap();
    let retired = ledger
        .read_event_page(0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "runSessionRetired")
        .count();
    assert_eq!(retired, 1);
}
