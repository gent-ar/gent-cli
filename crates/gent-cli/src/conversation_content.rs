//! Protocol-only client for private, paginated local user-prompt content.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_CONTENT_CAPABILITY, ConversationContentFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{ConversationContentCursor, ConversationContentPage};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    before: Option<ConversationContentCursor>,
    limit: u16,
) -> Result<ConversationContentPage, Box<dyn std::error::Error>> {
    if limit == 0 {
        return Err("conversation content limit must be at least 1".into());
    }
    if let Some(cursor) = &before {
        cursor.ordinal_for(&conversation_id)?;
    }
    let expected_conversation_id = conversation_id.clone();
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == CONVERSATION_CONTENT_CAPABILITY)
    {
        return Err("daemon does not support private conversation content; upgrade gentd".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationContentFrame::Request {
            conversation_id,
            before,
            limit,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationContentFrame::Page(page)) = serde_json::from_value(raw.clone()) {
        if page.conversation_id != expected_conversation_id {
            return Err("daemon returned conversation content for another conversation".into());
        }
        if let Some(cursor) = &page.next_before {
            cursor.ordinal_for(&expected_conversation_id)?;
        }
        return Ok(page);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation content".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{
        CONVERSATION_CONTENT_CAPABILITY, ConversationContentFrame, Hello, Negotiated, WireFrame,
        read_frame, read_json_frame, write_frame, write_json_frame,
    };
    use gent_types::{
        CapabilitySet, ConversationContentCursor, ConversationContentEntry,
        ConversationContentPage, PROTOCOL_MAX,
    };
    use tokio::net::UnixListener;

    use super::request;

    #[tokio::test]
    async fn request_rejects_a_zero_limit_before_connecting() {
        let directory = tempfile::tempdir().unwrap();
        let error = request(
            Some(directory.path().into()),
            true,
            "conversation-1".into(),
            None,
            0,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("at least 1"));
    }

    #[tokio::test]
    async fn request_rejects_content_for_another_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { .. })
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![CONVERSATION_CONTENT_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            let _ = read_json_frame::<_, ConversationContentFrame>(&mut stream)
                .await
                .unwrap();
            write_json_frame(
                &mut stream,
                &ConversationContentFrame::Page(ConversationContentPage {
                    conversation_id: "conversation-2".into(),
                    entries: Vec::new(),
                    next_before: None,
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(
                Some(directory.path().into()),
                true,
                "conversation-1".into(),
                None,
                20,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("another conversation")
        );
    }

    #[tokio::test]
    async fn request_rejects_a_cursor_bound_to_another_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![CONVERSATION_CONTENT_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            let _ = read_json_frame::<_, ConversationContentFrame>(&mut stream)
                .await
                .unwrap();
            write_json_frame(
                &mut stream,
                &ConversationContentFrame::Page(ConversationContentPage {
                    conversation_id: "conversation-1".into(),
                    entries: vec![ConversationContentEntry {
                        message_id: "message-1".into(),
                        turn_id: "turn-1".into(),
                        run_id: "run-1".into(),
                        ordinal: 1,
                        text: "prompt".into(),
                        text_digest_sha256: "a".repeat(64),
                    }],
                    next_before: Some(ConversationContentCursor::new("conversation-2", 1)),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(
                Some(directory.path().into()),
                true,
                "conversation-1".into(),
                None,
                20,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("cursor is invalid")
        );
    }

    #[tokio::test]
    async fn request_rejects_a_cross_conversation_cursor_before_connecting() {
        let directory = tempfile::tempdir().unwrap();
        let error = request(
            Some(directory.path().into()),
            true,
            "conversation-1".into(),
            Some(ConversationContentCursor::new("conversation-2", 1)),
            20,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("cursor is invalid"));
    }
}
