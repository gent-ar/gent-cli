use gent_ports::{Ledger, RunCheckpointLedger, RunRecord};
use gent_store::SqliteLedger;
use gent_types::RunCheckpointRecord;

fn checkpoint(sequence: u64, cursor: u64) -> RunCheckpointRecord {
    RunCheckpointRecord {
        checkpoint_id: format!("checkpoint-{sequence}"),
        run_id: "run-a".into(),
        sequence,
        event_cursor: cursor,
        state_digest_sha256: "a".repeat(64),
    }
}

fn prepare(ledger: &SqliteLedger) {
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
}

#[test]
fn checkpoints_are_ordered_monotonic_and_durable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    prepare(&ledger);
    ledger.save_run_checkpoint(&checkpoint(1, 4)).unwrap();
    ledger.save_run_checkpoint(&checkpoint(2, 4)).unwrap();
    assert!(ledger.save_run_checkpoint(&checkpoint(2, 5)).is_err());
    assert!(ledger.save_run_checkpoint(&checkpoint(3, 3)).is_err());
    drop(ledger);
    assert_eq!(
        SqliteLedger::open(path)
            .unwrap()
            .list_run_checkpoints("run-a")
            .unwrap(),
        vec![checkpoint(1, 4), checkpoint(2, 4)]
    );
}

#[test]
fn checkpoints_require_a_known_run_and_sha256_digest() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(ledger.save_run_checkpoint(&checkpoint(1, 1)).is_err());
    prepare(&ledger);
    assert!(
        ledger
            .save_run_checkpoint(&RunCheckpointRecord {
                state_digest_sha256: "not-a-digest".into(),
                ..checkpoint(1, 1)
            })
            .is_err()
    );
}
