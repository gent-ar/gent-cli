//! Private local user-prompt content with conversation-bound pagination cursors.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

/// One locally readable user-authored message. Assistant output is not modeled yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContentEntry {
    pub message_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub ordinal: u64,
    pub text: String,
    pub text_digest_sha256: String,
}

/// Opaque cursor bound to one conversation and an immutable ordinal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ConversationContentCursor(String);

impl ConversationContentCursor {
    #[must_use]
    pub fn new(conversation_id: &str, ordinal: u64) -> Self {
        let value = format!("v1\0{conversation_id}\0{ordinal}");
        Self(URL_SAFE_NO_PAD.encode(value))
    }

    /// Returns the cursor ordinal only when it belongs to `conversation_id`.
    ///
    /// # Errors
    /// Returns an error for malformed, unsupported, or cross-conversation cursors.
    pub fn ordinal_for(
        &self,
        conversation_id: &str,
    ) -> Result<u64, ConversationContentCursorError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| ConversationContentCursorError)?;
        let value = std::str::from_utf8(&bytes).map_err(|_| ConversationContentCursorError)?;
        let mut parts = value.split('\0');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("v1"), Some(found), Some(ordinal), None) if found == conversation_id => ordinal
                .parse()
                .ok()
                .filter(|ordinal: &u64| *ordinal > 0)
                .ok_or(ConversationContentCursorError),
            _ => Err(ConversationContentCursorError),
        }
    }
}

impl std::str::FromStr for ConversationContentCursor {
    type Err = ConversationContentCursorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let cursor = Self(value.into());
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| ConversationContentCursorError)?;
        Ok(cursor)
    }
}

impl std::fmt::Display for ConversationContentCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Cursor validation deliberately reveals no underlying protocol detail.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
#[error("conversation content cursor is invalid for this conversation")]
pub struct ConversationContentCursorError;

/// Newest-first local page; `next_before` is set only when more entries exist.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContentPage {
    pub conversation_id: String,
    pub entries: Vec<ConversationContentEntry>,
    pub next_before: Option<ConversationContentCursor>,
}

#[cfg(test)]
mod tests {
    use super::ConversationContentCursor;

    #[test]
    fn cursor_is_opaque_and_conversation_bound() {
        let cursor = ConversationContentCursor::new("conversation-1", 3);
        assert!(
            !serde_json::to_string(&cursor)
                .unwrap()
                .contains("conversation-1")
        );
        assert_eq!(cursor.ordinal_for("conversation-1"), Ok(3));
        assert!(cursor.ordinal_for("conversation-2").is_err());
    }
}
