//! Durable, provenance-bearing title and recap records.

use serde::{Deserialize, Serialize};

/// The user-visible content generated for a conversation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationArtifactKind {
    Title,
    Recap,
}

/// Lifecycle of an immutable generated-content attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationArtifactStatus {
    Pending,
    Completed,
    Failed,
    Superseded,
}

/// Provenance and result for one title or recap generation attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationArtifact {
    pub artifact_id: String,
    pub conversation_id: String,
    pub kind: ConversationArtifactKind,
    pub source_turn_ids: Vec<String>,
    pub provider: String,
    pub model_version: String,
    pub input_digest: String,
    pub status: ConversationArtifactStatus,
    pub text: Option<String>,
    pub supersedes_artifact_id: Option<String>,
}
