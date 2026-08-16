use gent_ports::LedgerError;
use gent_ports::runtime_update::RuntimeUpdateJournal;
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, RuntimeReleaseIdentity, RuntimeStagingReceipt, RuntimeUpdateHandoff,
    RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion,
};

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
        handoff: RuntimeUpdateHandoff::default(),
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

#[test]
fn runtime_update_handoff_facts_are_immutable_after_planning() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut planned = record(1);
    planned.handoff = RuntimeUpdateHandoff {
        origin_host_epoch: Some(HostEpoch(7)),
        release: Some(RuntimeReleaseIdentity {
            key_id: "release-key".into(),
            release_version: RuntimeVersion {
                major: 1,
                minor: 2,
                patch: 3,
            },
            target: "fixture-target".into(),
            artifact_digest_sha256: planned.artifact_digest_sha256.clone(),
        }),
        staging_receipt: None,
    };
    ledger.save_runtime_update(&planned).unwrap();
    let mut staged = planned.clone();
    staged.revision = 2;
    staged.handoff.staging_receipt = Some(RuntimeStagingReceipt {
        attempt_id: staged.attempt_id.clone(),
        artifact_digest_sha256: staged.artifact_digest_sha256.clone(),
    });
    ledger.save_runtime_update(&staged).unwrap();
    let mut changed_epoch = staged.clone();
    changed_epoch.revision = 3;
    changed_epoch.handoff.origin_host_epoch = Some(HostEpoch(8));
    assert!(matches!(
        ledger.save_runtime_update(&changed_epoch),
        Err(LedgerError::Invariant(_))
    ));
    let mut changed_receipt = staged;
    changed_receipt.revision = 3;
    changed_receipt.handoff.staging_receipt = None;
    assert!(matches!(
        ledger.save_runtime_update(&changed_receipt),
        Err(LedgerError::Invariant(_))
    ));
}
