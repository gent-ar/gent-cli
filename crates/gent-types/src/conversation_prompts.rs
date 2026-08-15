//! User-authored conversation prompt values retained outside event payloads.

use serde::{Deserialize, Serialize};

/// One requested user prompt before it is assigned a durable turn sequence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPrompt {
    pub message_id: String,
    pub turn_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub text: String,
}

/// A durable user prompt linked to one conversation turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub message_id: String,
    pub turn_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub sequence: u64,
    pub text: String,
    pub text_digest_sha256: String,
}
