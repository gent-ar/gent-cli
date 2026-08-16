use gent_ports::runtime_update::RuntimeUpdateJournal;
use gent_ports::{IngressMode, Ledger};
use gent_runtime::{
    RuntimeUpdateAuthority, RuntimeUpdateSuccessor, RuntimeUpdateSuccessorError,
    RuntimeUpdateSuccessorRequest, RuntimeUpdateSuccessorResult,
};
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, RuntimeReleaseIdentity, RuntimeStagingReceipt, RuntimeUpdateHandoff,
    RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion,
};

fn release() -> RuntimeReleaseIdentity {
    RuntimeReleaseIdentity {
        key_id: "release-key".into(),
        release_version: RuntimeVersion {
            major: 1,
            minor: 2,
            patch: 3,
        },
        target: "aarch64-apple-darwin".into(),
        artifact_digest_sha256: "a".repeat(64),
    }
}

fn receipt() -> RuntimeStagingReceipt {
    RuntimeStagingReceipt {
        attempt_id: "attempt-1".into(),
        artifact_digest_sha256: "a".repeat(64),
    }
}

fn handoff_record() -> RuntimeUpdateRecord {
    RuntimeUpdateRecord {
        attempt_id: "attempt-1".into(),
        revision: 1,
        artifact_digest_sha256: "a".repeat(64),
        status: RuntimeUpdateStatus {
            stage: RuntimeUpdateStage::HandoffRequested,
            release_version: Some(release().release_version),
            forward_only_schema: false,
            failure: None,
        },
        handoff: RuntimeUpdateHandoff {
            origin_host_epoch: Some(HostEpoch(1)),
            release: Some(release()),
            staging_receipt: Some(receipt()),
        },
    }
}

fn request() -> RuntimeUpdateSuccessorRequest {
    RuntimeUpdateSuccessorRequest {
        attempt_id: "attempt-1".into(),
        active_host_epoch: HostEpoch(1),
        release: release(),
        staging_receipt: receipt(),
    }
}

fn successor(
    authority: RuntimeUpdateAuthority,
    ledger: SqliteLedger,
) -> RuntimeUpdateSuccessor<SqliteLedger> {
    RuntimeUpdateSuccessor::new(ledger, authority)
}

fn closed_handoff() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.save_runtime_update(&handoff_record()).unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger
}

#[test]
fn observer_successor_never_reads_or_writes_a_handoff() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let result = successor(RuntimeUpdateAuthority::Observer, ledger.clone())
        .confirm(&request())
        .unwrap();
    assert_eq!(result, RuntimeUpdateSuccessorResult::DeniedObserver);
    assert!(ledger.find_runtime_update("attempt-1").unwrap().is_none());
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
}

#[test]
fn mismatched_successor_release_rejects_without_a_checkpoint() {
    let ledger = closed_handoff();
    let mut wrong = request();
    wrong.release.key_id = "other-key".into();
    let error = successor(RuntimeUpdateAuthority::Approved, ledger.clone())
        .confirm(&wrong)
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeUpdateSuccessorError::ReleaseMismatch
    ));
    assert_eq!(
        ledger.find_runtime_update("attempt-1").unwrap(),
        Some(handoff_record())
    );
}

#[test]
fn exact_successor_confirms_activation_while_ingress_remains_closed() {
    let ledger = closed_handoff();
    let result = successor(RuntimeUpdateAuthority::Approved, ledger.clone())
        .confirm(&request())
        .unwrap();
    let RuntimeUpdateSuccessorResult::Confirmed(record) = result else {
        panic!("expected confirmation")
    };
    assert_eq!(record.revision, 2);
    assert_eq!(record.status.stage, RuntimeUpdateStage::Activated);
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    assert_eq!(
        ledger.find_runtime_update("attempt-1").unwrap(),
        Some(*record)
    );
}

#[test]
fn repeating_an_exact_confirmation_is_read_only() {
    let ledger = closed_handoff();
    let service = successor(RuntimeUpdateAuthority::Approved, ledger.clone());
    let first = service.confirm(&request()).unwrap();
    let second = service.confirm(&request()).unwrap();
    assert_eq!(second, first);
    assert_eq!(
        ledger
            .find_runtime_update("attempt-1")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}
