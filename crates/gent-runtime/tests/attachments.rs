use gent_runtime::AttachmentService;
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::{AttachmentMetadata, AttachmentState, AttachmentTransfer, HostEpoch, ReceiptId};
use sha2::{Digest, Sha256};

#[test]
fn staging_retries_then_commits_only_verified_content() {
    let directory = tempfile::tempdir().unwrap();
    let digest = format!("{:x}", Sha256::digest(b"hello"));
    let transfer = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: "attachment-1".into(),
            display_name: "notes.txt".into(),
            media_type: "text/plain".into(),
            byte_len: 5,
            digest_sha256: digest.clone(),
            storage_key: format!("sha256/{digest}"),
        },
        staging_key: "staging/attachment-1".into(),
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "attachment-1".into(),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    let service = AttachmentService::new(
        SqliteLedger::in_memory().unwrap(),
        FileAttachmentBlobs::open(directory.path()).unwrap(),
    );
    service.begin(&transfer).unwrap();
    service.append("attachment-1", 0, b"hello").unwrap();
    service.append("attachment-1", 0, b"hello").unwrap();
    assert_eq!(
        service.commit("attachment-1").unwrap().state,
        AttachmentState::Available
    );
    assert_eq!(
        service.commit("attachment-1").unwrap().state,
        AttachmentState::Available
    );
}

#[test]
fn separate_transfers_can_converge_on_one_content_address() {
    let directory = tempfile::tempdir().unwrap();
    let digest = format!("{:x}", Sha256::digest(b"same"));
    let service = AttachmentService::new(
        SqliteLedger::in_memory().unwrap(),
        FileAttachmentBlobs::open(directory.path()).unwrap(),
    );
    for id in ["attachment-1", "attachment-2"] {
        let transfer = AttachmentTransfer {
            metadata: AttachmentMetadata {
                attachment_id: id.into(),
                display_name: format!("{id}.txt"),
                media_type: "text/plain".into(),
                byte_len: 4,
                digest_sha256: digest.clone(),
                storage_key: format!("sha256/{digest}"),
            },
            staging_key: format!("staging/{id}"),
            receipt_id: ReceiptId(format!("receipt-{id}")),
            idempotency_key: id.into(),
            host_epoch: HostEpoch(1),
            state: AttachmentState::Uploading,
            received_bytes: 0,
        };
        service.begin(&transfer).unwrap();
        service.append(id, 0, b"same").unwrap();
        assert_eq!(
            service.commit(id).unwrap().state,
            AttachmentState::Available
        );
    }
    assert!(directory.path().join("blobs").join(digest).exists());
}
