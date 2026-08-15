use gent_ports::LedgerError;
use gent_ports::runtime_update::RuntimeUpdateJournal;
use gent_store::SqliteLedger;
use gent_types::{RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion};

fn record(revision: u64) -> RuntimeUpdateRecord {
    RuntimeUpdateRecord {
        attempt_id: "attempt".into(),
        revision,
        artifact_digest_sha256: "a".repeat(64),
        status: RuntimeUpdateStatus {
            stage: RuntimeUpdateStage::Available,
            release_version: Some(RuntimeVersion {
                major: 1,
                minor: 2,
                patch: 3,
            }),
            forward_only_schema: false,
            failure: None,
        },
    }
}

#[test]
fn runtime_update_checkpoints_survive_restart_at_the_latest_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.save_runtime_update(&record(1)).unwrap();
    let latest = record(2);
    ledger.save_runtime_update(&latest).unwrap();
    drop(ledger);

    assert_eq!(
        SqliteLedger::open(path)
            .unwrap()
            .find_runtime_update("attempt")
            .unwrap(),
        Some(latest)
    );
}

#[test]
fn runtime_update_retries_are_idempotent_and_revisions_are_append_only() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let latest = record(2);
    ledger.save_runtime_update(&latest).unwrap();
    ledger.save_runtime_update(&latest).unwrap();
    assert!(matches!(
        ledger.save_runtime_update(&record(1)),
        Err(LedgerError::Invariant(_))
    ));
    let mut conflict = latest;
    conflict.status.stage = RuntimeUpdateStage::Staged;
    assert!(matches!(
        ledger.save_runtime_update(&conflict),
        Err(LedgerError::Invariant(_))
    ));
}

#[test]
fn runtime_update_rejects_missing_digest_or_revision() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut invalid = record(1);
    invalid.revision = 0;
    assert!(matches!(
        ledger.save_runtime_update(&invalid),
        Err(LedgerError::Invariant(_))
    ));
    invalid = record(1);
    invalid.artifact_digest_sha256.clear();
    assert!(matches!(
        ledger.save_runtime_update(&invalid),
        Err(LedgerError::Invariant(_))
    ));
}
