//! Protocol-only client request for content-free conversation discovery.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_INDEX_CAPABILITY, ConversationIndexFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::ConversationListItem;
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

/// Reads selectable conversation identities and run counts without exposing message content.
///
/// # Errors
/// Returns an error when the daemon is unavailable, unnegotiated, or lacks the capability.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<Vec<ConversationListItem>, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == CONVERSATION_INDEX_CAPABILITY)
    {
        return Err("daemon does not support conversation discovery; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &ConversationIndexFrame::Request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationIndexFrame::Index(index)) = serde_json::from_value(raw.clone()) {
        return Ok(index);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a conversation index".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{
        CONVERSATION_INDEX_CAPABILITY, ConversationIndexFrame, Hello, Negotiated, WireFrame,
        read_frame, read_json_frame, write_frame, write_json_frame,
    };
    use gent_types::{CapabilitySet, ConversationListItem, PROTOCOL_MAX};
    use tokio::net::UnixListener;

    use super::request;

    #[tokio::test]
    async fn request_requires_capability_and_returns_content_free_items() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == CONVERSATION_INDEX_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![CONVERSATION_INDEX_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, ConversationIndexFrame>(&mut stream)
                    .await
                    .unwrap(),
                ConversationIndexFrame::Request
            ));
            write_json_frame(
                &mut stream,
                &ConversationIndexFrame::Index(vec![ConversationListItem {
                    conversation_id: "conversation-1".into(),
                    run_count: 2,
                }]),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            request(Some(directory.path().into()), true).await.unwrap(),
            vec![ConversationListItem {
                conversation_id: "conversation-1".into(),
                run_count: 2,
            }]
        );
    }

    #[tokio::test]
    async fn request_rejects_a_daemon_without_the_negotiated_capability() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(Some(directory.path().into()), true)
                .await
                .unwrap_err()
                .to_string()
                .contains("upgrade gentd")
        );
    }
}
