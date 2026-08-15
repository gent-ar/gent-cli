//! Negotiated, local-only reads of durable user prompt content.

use gent_types::{ConversationContentCursor, ConversationContentPage};
use serde::{Deserialize, Serialize};

pub const CONVERSATION_CONTENT_CAPABILITY: &str = "conversation-content-v1";
pub const MAX_CONVERSATION_CONTENT_PAGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ConversationContentFrame {
    Request {
        conversation_id: String,
        before: Option<ConversationContentCursor>,
        limit: u16,
    },
    Page(ConversationContentPage),
}

/// Bounds a page by its exact JSON protocol representation while preserving keyset pagination.
///
/// # Errors
/// Returns an opaque error when one entry cannot fit in a readable page.
pub fn bound_content_page(
    mut page: ConversationContentPage,
) -> Result<ConversationContentPage, ContentPageError> {
    if fits(&page)? {
        return Ok(page);
    }
    while page.entries.pop().is_some() {
        if let Some(tail) = page.entries.last() {
            page.next_before = Some(ConversationContentCursor::new(
                &page.conversation_id,
                tail.ordinal,
            ));
            if fits(&page)? {
                return Ok(page);
            }
        }
    }
    Err(ContentPageError)
}

fn fits(page: &ConversationContentPage) -> Result<bool, ContentPageError> {
    serde_json::to_vec(&ConversationContentFrame::Page(page.clone()))
        .map(|encoded| encoded.len() <= MAX_CONVERSATION_CONTENT_PAGE_BYTES)
        .map_err(|_| ContentPageError)
}

/// Content-size failures deliberately omit any user content.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
#[error("conversation content page exceeds protocol byte budget")]
pub struct ContentPageError;

#[cfg(test)]
mod tests {
    use super::{
        ConversationContentFrame, MAX_CONVERSATION_CONTENT_PAGE_BYTES, bound_content_page,
    };
    use gent_types::{
        ConversationContentCursor, ConversationContentEntry, ConversationContentPage,
    };

    #[test]
    fn oversized_page_is_trimmed_with_a_cursor_for_its_retained_tail() {
        let page = ConversationContentPage {
            conversation_id: "conversation".into(),
            entries: (1..=20).rev().map(entry).collect(),
            next_before: None,
        };
        let bounded = bound_content_page(page).unwrap();
        assert!(
            serde_json::to_vec(&ConversationContentFrame::Page(bounded.clone()))
                .unwrap()
                .len()
                <= MAX_CONVERSATION_CONTENT_PAGE_BYTES
        );
        assert!(bounded.entries.len() < 20);
        assert_eq!(
            bounded.next_before.unwrap().ordinal_for("conversation"),
            Ok(bounded.entries.last().unwrap().ordinal)
        );
    }

    #[test]
    fn page_that_cannot_advance_is_rejected_without_content() {
        let page = ConversationContentPage {
            conversation_id: "conversation".into(),
            entries: vec![ConversationContentEntry {
                text: "x".repeat(MAX_CONVERSATION_CONTENT_PAGE_BYTES),
                ..entry(1)
            }],
            next_before: None,
        };
        assert!(bound_content_page(page).is_err());
    }

    fn entry(ordinal: u64) -> ConversationContentEntry {
        ConversationContentEntry {
            message_id: format!("message-{ordinal}"),
            turn_id: format!("turn-{ordinal}"),
            run_id: "run".into(),
            ordinal,
            text: "x".repeat(128 * 1024),
            text_digest_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn terminal_page_within_budget_is_unchanged() {
        let page = ConversationContentPage {
            conversation_id: "conversation".into(),
            entries: vec![ConversationContentEntry {
                message_id: "message".into(),
                turn_id: "turn".into(),
                run_id: "run".into(),
                ordinal: 1,
                text: "hello".into(),
                text_digest_sha256: "a".repeat(64),
            }],
            next_before: None,
        };
        assert_eq!(bound_content_page(page.clone()).unwrap(), page);
    }

    #[test]
    fn existing_cursor_is_replaced_when_a_page_is_trimmed() {
        let page = ConversationContentPage {
            conversation_id: "conversation".into(),
            entries: (1..=20).rev().map(entry).collect(),
            next_before: Some(ConversationContentCursor::new("conversation", 1)),
        };
        let bounded = bound_content_page(page).unwrap();
        assert_eq!(
            bounded.next_before.unwrap().ordinal_for("conversation"),
            Ok(bounded.entries.last().unwrap().ordinal)
        );
    }
}
