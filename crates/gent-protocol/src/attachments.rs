//! Capability-gated local attachment staging frames; they never carry source paths.

use gent_types::{AttachmentOperation, AttachmentTransfer};
use serde::{Deserialize, Serialize};

/// Negotiates local daemon-owned attachment staging, not provider injection.
pub const ATTACHMENTS_CAPABILITY: &str = "attachments-v1";

/// Additive attachment transfer frames used after the regular hello negotiation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum AttachmentFrame {
    Begin {
        transfer: AttachmentTransfer,
    },
    Chunk {
        operation: AttachmentOperation,
        offset: u64,
        data_base64: String,
    },
    Commit {
        operation: AttachmentOperation,
    },
    Resume {
        attachment_id: String,
    },
    Transfer {
        transfer: AttachmentTransfer,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ATTACHMENTS_CAPABILITY, AttachmentFrame};

    #[test]
    fn chunk_contract_has_no_source_path_field() {
        let frame = AttachmentFrame::Chunk {
            operation: gent_types::AttachmentOperation {
                attachment_id: "attachment-1".into(),
                receipt_id: gent_types::ReceiptId("receipt-1".into()),
                idempotency_key: "attachment-1".into(),
                host_epoch: gent_types::HostEpoch(1),
            },
            offset: 0,
            data_base64: "aGVsbG8=".into(),
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["type"], "chunk");
        assert!(value["body"].get("path").is_none());
        assert_eq!(value["body"]["operation"]["receiptId"], "receipt-1");
        assert_eq!(ATTACHMENTS_CAPABILITY, "attachments-v1");
    }
}
