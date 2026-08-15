//! Capability-gated local attachment staging frames; they never carry source paths.

use gent_types::AttachmentTransfer;
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
        attachment_id: String,
        offset: u64,
        data_base64: String,
    },
    Commit {
        attachment_id: String,
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
            attachment_id: "attachment-1".into(),
            offset: 0,
            data_base64: "aGVsbG8=".into(),
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["type"], "chunk");
        assert!(value["body"].get("path").is_none());
        assert_eq!(ATTACHMENTS_CAPABILITY, "attachments-v1");
    }
}
