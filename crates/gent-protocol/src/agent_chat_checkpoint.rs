//! Negotiated local IPC for durable per-turn file checkpoints and their restore.

use gent_types::{AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatFileSnapshot};
use serde::{Deserialize, Serialize};

/// Negotiated capability for local checkpoint capture, listing, and restore.
pub const AGENT_CHAT_CHECKPOINT_CAPABILITY: &str = "agent-chat-checkpoint-v1";

/// One finite checkpoint exchange. Provider execution is never part of this protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatCheckpointFrame {
    CaptureCheckpoint {
        request_id: String,
        receipt_id: String,
        conversation_id: String,
        run_id: String,
        message_ordinal: u64,
        files: Vec<AgentChatFileSnapshot>,
    },
    Captured {
        request_id: String,
        checkpoint: AgentChatFileCheckpoint,
    },
    ListCheckpoints {
        request_id: String,
        conversation_id: String,
    },
    Checkpoints {
        request_id: String,
        checkpoints: Vec<AgentChatFileCheckpoint>,
    },
    RestoreCheckpoint {
        request_id: String,
        receipt_id: String,
        conversation_id: String,
        checkpoint_id: String,
        restore_files: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restore_files_confirmation: Option<String>,
    },
    Restored {
        request_id: String,
        conversation_id: String,
        checkpoint_id: String,
        run_id: String,
        visible_through_ordinal: u64,
        restored_files: Vec<AgentChatFileCheckpointFile>,
    },
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_CHECKPOINT_CAPABILITY, AgentChatCheckpointFrame};
    use serde_json::json;

    #[test]
    fn restore_frame_omits_the_confirmation_when_absent() {
        let frame = AgentChatCheckpointFrame::RestoreCheckpoint {
            request_id: "request-1".into(),
            receipt_id: "receipt-1".into(),
            conversation_id: "conversation-1".into(),
            checkpoint_id: "checkpoint-1".into(),
            restore_files: false,
            restore_files_confirmation: None,
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({
                "type": "restoreCheckpoint",
                "body": {
                    "requestId": "request-1", "receiptId": "receipt-1",
                    "conversationId": "conversation-1", "checkpointId": "checkpoint-1",
                    "restoreFiles": false
                }
            })
        );
        assert_eq!(AGENT_CHAT_CHECKPOINT_CAPABILITY, "agent-chat-checkpoint-v1");
    }

    #[test]
    fn frame_rejects_unknown_fields() {
        let frame = json!({
            "type": "listCheckpoints",
            "body": { "requestId": "request-1", "conversationId": "conversation-1", "extra": true }
        });
        assert!(serde_json::from_value::<AgentChatCheckpointFrame>(frame).is_err());
    }
}
