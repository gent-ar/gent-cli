use std::sync::{Arc, Barrier};
use std::thread;

use gent_core::Run;
use gent_ports::{LeaseClaim, RunLease, RunLeaseClaim, WorktreeLease};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, Command, HostEpoch, ReceiptId};
use serde_json::json;

fn coordinator() -> Coordinator<SqliteLedger> {
    Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default())
}

#[test]
fn concurrent_coordinators_have_one_run_lease_winner() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ledger.sqlite");
    let first = Coordinator::new(
        SqliteLedger::open(&database).unwrap(),
        CapabilitySet::default(),
    );
    let second = Coordinator::new(
        SqliteLedger::open(&database).unwrap(),
        CapabilitySet::default(),
    );
    first
        .create_run(Run {
            id: "run".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let contenders = [(first, "first"), (second, "second")].map(|(coordinator, id)| {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            coordinator
                .claim_run_lease(&RunLease {
                    run_id: "run".into(),
                    coordinator_id: id.into(),
                    host_epoch: HostEpoch(1),
                })
                .unwrap()
        })
    });
    let results = contenders.map(|thread| thread.join().unwrap());
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, RunLeaseClaim::Acquired(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, RunLeaseClaim::Contended(_)))
            .count(),
        1
    );
}

#[test]
fn concurrent_worktree_claims_have_one_winner() {
    let coordinator = coordinator();
    coordinator
        .create_run(Run {
            id: "run".into(),
            parent_run_id: None,
            provider: "codex".into(),
        })
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let contenders = ["first", "second"].map(|token| {
        let coordinator = coordinator.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            coordinator
                .claim_worktree_lease(&WorktreeLease {
                    worktree_id: "tree".into(),
                    run_id: "run".into(),
                    lease_token: token.into(),
                    host_epoch: HostEpoch(1),
                })
                .unwrap()
        })
    });
    let results = contenders.map(|thread| thread.join().unwrap());
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, LeaseClaim::Acquired(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, LeaseClaim::Contended(_)))
            .count(),
        1
    );
}

#[test]
fn concurrent_command_retries_produce_one_receipt_and_event_pair() {
    let coordinator = coordinator();
    let barrier = Arc::new(Barrier::new(2));
    let receipts = [ReceiptId::new(), ReceiptId::new()].map(|receipt_id| {
        let coordinator = coordinator.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            coordinator
                .submit(&Command {
                    receipt_id,
                    idempotency_key: "same-command".into(),
                    host_epoch: HostEpoch(1),
                    kind: "ping".into(),
                    payload: json!({"concurrent": true}),
                })
                .unwrap()
        })
    });
    let receipts = receipts.map(|thread| thread.join().unwrap());
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(coordinator.events_after(0).unwrap().len(), 2);
}
