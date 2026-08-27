use std::sync::{Arc, Barrier};
use std::thread;

use gent_core::Run;
use gent_ports::{LeaseClaim, Ledger, RunLease, RunLeaseClaim, WorktreeLease};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, Command, HostEpoch, ReceiptId, RunVersionLock};
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
        .create_run(&Run {
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
        .create_run(&Run {
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
    let command = Command {
        receipt_id: ReceiptId::new(),
        idempotency_key: "same-command".into(),
        host_epoch: HostEpoch(1),
        kind: "ping".into(),
        payload: json!({"concurrent": true}),
    };
    let receipts = [command.clone(), command].map(|command| {
        let coordinator = coordinator.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            coordinator.submit(&command).unwrap()
        })
    });
    let receipts = receipts.map(|thread| thread.join().unwrap());
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(coordinator.read_event_page(0, 100).unwrap().events.len(), 2);
}

#[test]
fn run_version_lock_is_immutable_and_durable() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    coordinator
        .create_run(&Run {
            id: "run".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    let lock = RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/tool/claude".into(),
        file_identity: "identity".into(),
        digest_sha256: "digest".into(),
        version: "1.0".into(),
        compatibility_entry: "claude-1".into(),
    };
    coordinator.lock_run_version("run", &lock).unwrap();
    assert_eq!(
        ledger.find_run_version_lock("run").unwrap(),
        Some(lock.clone())
    );
    assert!(coordinator.lock_run_version("run", &lock).is_err());
}

#[test]
fn file_backed_ledger_preserves_events_and_epoch_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ledger.sqlite");
    let first = Coordinator::new(
        SqliteLedger::open(&database).unwrap(),
        CapabilitySet::default(),
    );
    let receipt = first
        .submit(&Command {
            receipt_id: ReceiptId::new(),
            idempotency_key: "restart".into(),
            host_epoch: HostEpoch(1),
            kind: "ping".into(),
            payload: json!({"restart": true}),
        })
        .unwrap();
    drop(first);

    let restarted = Coordinator::new(
        SqliteLedger::open(&database).unwrap(),
        CapabilitySet::default(),
    );
    assert_eq!(restarted.status().unwrap().host_epoch, HostEpoch(1));
    let events = restarted.read_event_page(0, 100).unwrap().events;
    assert_eq!(events.len(), 2);
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].cursor < pair[1].cursor)
    );
    assert!(
        events
            .iter()
            .all(|event| event.receipt_id == receipt.receipt_id)
    );
}
