//! Typed capture/restore for durable per-turn file snapshots.
//!
//! Named `AgentChatFileCheckpoint` throughout, never `Checkpoint`/`RunCheckpoint`, to avoid
//! collision with the unrelated internal event-sourcing `RunCheckpointRecord`.

use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, HostEpoch, Receipt, ReceiptId,
};

/// One file's content, as read from the workspace immediately before a tool edits it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatFileSnapshot {
    pub file_path: String,
    pub content: String,
}

/// A captured file's durable, content-addressed storage identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatFileCheckpointFile {
    pub file_path: String,
    pub storage_key: String,
    pub byte_len: u64,
}

/// One immutable durable checkpoint: the files a turn was about to change, before it changed them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatFileCheckpoint {
    pub checkpoint_id: String,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub message_ordinal: u64,
    pub created_at_unix_ms: u64,
    pub files: Vec<AgentChatFileCheckpointFile>,
}

/// Client correlation and content required to capture one checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatCheckpointCapture {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub message_ordinal: u64,
    pub created_at_unix_ms: u64,
    pub files: Vec<AgentChatFileSnapshot>,
}

/// Client correlation required to restore a durable checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatCheckpointRestore {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub checkpoint_id: String,
    pub restore_files: bool,
    /// Required non-empty whenever `restore_files` is true; never trusted from the client alone.
    pub restore_files_confirmation: Option<String>,
}

/// A retry-stable new run whose context resumes exactly at the restored checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatCheckpointRestored {
    pub receipt: Receipt,
    pub conversation_id: AgentChatConversationId,
    pub checkpoint_id: String,
    pub run_id: AgentChatRunId,
    pub visible_through_ordinal: u64,
    pub restored_files: Vec<AgentChatFileCheckpointFile>,
}
