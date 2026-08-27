//! Attachment-frame adapter over the runtime's opaque staging service.

use base64::Engine;
use gent_protocol::AttachmentFrame;
use gent_runtime::AttachmentService;
use gent_types::HostEpoch;

/// Executes one negotiated attachment request without exposing blob paths to transport code.
pub(crate) fn handle(
    attachments: &AttachmentService<gent_store::SqliteLedger, gent_store::FileAttachmentBlobs>,
    host_epoch: HostEpoch,
    frame: AttachmentFrame,
) -> Result<AttachmentFrame, String> {
    match frame {
        AttachmentFrame::Begin { mut transfer } => {
            transfer.host_epoch = host_epoch;
            attachments
                .begin(&transfer)
                .map_err(|error| error.to_string())
                .map(|transfer| AttachmentFrame::Transfer { transfer })
        }
        AttachmentFrame::Chunk {
            mut operation,
            offset,
            data_base64,
        } => {
            operation.host_epoch = attachments
                .resume(&operation.attachment_id)
                .map_err(|error| error.to_string())?
                .host_epoch;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|error| error.to_string())?;
            attachments
                .append(&operation, offset, &bytes)
                .map_err(|error| error.to_string())
                .map(|transfer| AttachmentFrame::Transfer { transfer })
        }
        AttachmentFrame::Commit { mut operation } => {
            operation.host_epoch = attachments
                .resume(&operation.attachment_id)
                .map_err(|error| error.to_string())?
                .host_epoch;
            attachments
                .commit(&operation)
                .map_err(|error| error.to_string())
                .map(|transfer| AttachmentFrame::Transfer { transfer })
        }
        AttachmentFrame::Resume { attachment_id } => attachments
            .resume(&attachment_id)
            .map_err(|error| error.to_string())
            .map(|transfer| AttachmentFrame::Transfer { transfer }),
        AttachmentFrame::Transfer { .. } | AttachmentFrame::Error { .. } => {
            Err("attachment response frames are not valid requests".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use gent_protocol::AttachmentFrame;
    use gent_runtime::AttachmentService;
    use gent_store::{FileAttachmentBlobs, SqliteLedger};
    use gent_types::{
        AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, HostEpoch,
        ReceiptId,
    };

    use super::handle;

    #[test]
    fn daemon_epoch_owns_every_attachment_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
        let attachments = AttachmentService::new(
            ledger,
            FileAttachmentBlobs::open(directory.path().join("attachments")).unwrap(),
        );
        let epoch = HostEpoch(1);
        let begin = transfer(HostEpoch(999));
        let AttachmentFrame::Transfer { transfer } = handle(
            &attachments,
            epoch,
            AttachmentFrame::Begin { transfer: begin },
        )
        .unwrap() else {
            panic!("begin must return a transfer");
        };
        assert_eq!(transfer.host_epoch, epoch);

        let AttachmentFrame::Transfer { transfer } = handle(
            &attachments,
            epoch,
            AttachmentFrame::Chunk {
                operation: operation(&transfer, HostEpoch(777), "append"),
                offset: 0,
                data_base64: base64::engine::general_purpose::STANDARD.encode(b"hello"),
            },
        )
        .unwrap() else {
            panic!("append must return a transfer");
        };
        assert_eq!(transfer.host_epoch, epoch);
        assert_eq!(transfer.received_bytes, 5);

        let AttachmentFrame::Transfer { transfer } = handle(
            &attachments,
            epoch,
            AttachmentFrame::Commit {
                operation: operation(&transfer, HostEpoch(555), "commit"),
            },
        )
        .unwrap() else {
            panic!("commit must return a transfer");
        };
        assert_eq!(transfer.host_epoch, epoch);
        assert_eq!(transfer.state, AttachmentState::Available);
    }

    fn transfer(host_epoch: HostEpoch) -> AttachmentTransfer {
        AttachmentTransfer {
            metadata: AttachmentMetadata {
                attachment_id: "attachment-1".into(),
                display_name: "hello.txt".into(),
                media_type: "text/plain".into(),
                byte_len: 5,
                digest_sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                    .into(),
                storage_key:
                    "sha256/2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
            },
            staging_key: "staging/attachment-1".into(),
            receipt_id: ReceiptId("begin-receipt".into()),
            idempotency_key: "begin-key".into(),
            host_epoch,
            state: AttachmentState::Uploading,
            received_bytes: 0,
        }
    }

    fn operation(
        transfer: &AttachmentTransfer,
        host_epoch: HostEpoch,
        suffix: &str,
    ) -> AttachmentOperation {
        AttachmentOperation {
            attachment_id: transfer.metadata.attachment_id.clone(),
            transfer_receipt_id: transfer.receipt_id.clone(),
            transfer_idempotency_key: transfer.idempotency_key.clone(),
            receipt_id: ReceiptId(format!("{suffix}-receipt")),
            idempotency_key: format!("{suffix}-key"),
            host_epoch,
        }
    }
}
