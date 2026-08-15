use gent_ports::{AttachmentClaim, AttachmentLedger, Ledger};
use gent_store::SqliteLedger;
use gent_types::{
    AttachmentMetadata, AttachmentState, AttachmentTransfer, HostEpoch, ReceiptId, TurnAttachment,
};

fn transfer() -> AttachmentTransfer {
    AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: "attachment-1".into(),
            display_name: "notes.txt".into(),
            media_type: "text/plain".into(),
            byte_len: 4,
            digest_sha256: "a".repeat(64),
            storage_key: format!("sha256/{}", "a".repeat(64)),
        },
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "attachment-1".into(),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    }
}

#[test]
fn transfer_progress_is_optimistic_idempotent_and_restart_safe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let initial = transfer();
    assert!(matches!(
        ledger.claim_attachment(&initial).unwrap(),
        AttachmentClaim::Created(_)
    ));
    assert!(matches!(
        ledger.claim_attachment(&initial).unwrap(),
        AttachmentClaim::Existing(_)
    ));
    let mut progressed = initial.clone();
    progressed.received_bytes = 4;
    ledger.replace_attachment(&initial, &progressed).unwrap();
    assert!(matches!(
        ledger.claim_attachment(&initial).unwrap(),
        AttachmentClaim::Existing(found) if found == progressed
    ));
    assert!(ledger.replace_attachment(&initial, &progressed).is_err());
    drop(ledger);
    assert_eq!(
        SqliteLedger::open(&path)
            .unwrap()
            .find_attachment("attachment-1")
            .unwrap(),
        Some(progressed)
    );
}

#[test]
fn conflicting_idempotency_metadata_is_rejected() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let initial = transfer();
    ledger.claim_attachment(&initial).unwrap();
    let mut conflict = initial;
    conflict.metadata.display_name = "other.txt".into();
    assert!(ledger.claim_attachment(&conflict).is_err());
}

#[test]
fn closed_or_fenced_host_rejects_attachment_progress() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(ledger.claim_attachment(&transfer()).is_err());
}

#[test]
fn association_is_fenced_and_attachment_updates_are_monotonic() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let initial = transfer();
    ledger.claim_attachment(&initial).unwrap();
    let mut uploaded = initial.clone();
    uploaded.received_bytes = 4;
    ledger.replace_attachment(&initial, &uploaded).unwrap();
    let mut available = uploaded.clone();
    available.state = AttachmentState::Available;
    assert!(ledger.replace_attachment(&uploaded, &available).is_ok());
    assert!(ledger.replace_attachment(&available, &initial).is_err());
    let association = TurnAttachment {
        turn_id: "turn-1".into(),
        attachment_id: initial.metadata.attachment_id,
        host_epoch: HostEpoch(2),
    };
    assert!(ledger.attach_to_turn(&association).is_err());
}
