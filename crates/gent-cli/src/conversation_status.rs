//! Protocol-only client request for durable, read-only conversation status.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_STATUS_CAPABILITY, ConversationStatusFrame, Hello, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{ConversationStatus, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::Value;

use crate::local_ipc::{client_capabilities, connect_or_start, default_data_dir};

/// Reads durable run and turn status without creating a command receipt.
///
/// # Errors
/// Returns an error when the daemon is unavailable, unnegotiated, or lacks the capability.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<ConversationStatus, Box<dyn std::error::Error>> {
    let expected_conversation_id = conversation_id.clone();
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir, no_autostart).await?;
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: client_capabilities(),
        }),
    )
    .await?;
    let WireFrame::Negotiated(negotiated) = read_frame(&mut stream).await? else {
        return Err("daemon did not negotiate protocol".into());
    };
    if !negotiated
        .capabilities
        .0
        .iter()
        .any(|capability| capability == CONVERSATION_STATUS_CAPABILITY)
    {
        return Err("daemon does not support conversation status; upgrade gentd".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationStatusFrame::Request { conversation_id },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationStatusFrame::Status(status)) = serde_json::from_value(raw.clone())
        && status.conversation_id == expected_conversation_id
    {
        return Ok(status);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation status".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{
        ConversationStatusFrame, Hello, Negotiated, read_frame, read_json_frame, write_frame,
        write_json_frame,
    };
    use gent_types::{CapabilitySet, ConversationStatus, PROTOCOL_MAX};
    use tokio::net::UnixListener;

    use super::request;

    #[tokio::test]
    async fn request_requires_capability_and_returns_only_a_status_frame() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                gent_protocol::WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == "conversation-status-v1")
            ));
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec!["conversation-status-v1".into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, ConversationStatusFrame>(&mut stream)
                    .await
                    .unwrap(),
                ConversationStatusFrame::Request { conversation_id } if conversation_id == "conversation-1"
            ));
            write_json_frame(
                &mut stream,
                &ConversationStatusFrame::Status(ConversationStatus {
                    conversation_id: "conversation-1".into(),
                    runs: Vec::new(),
                }),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            request(Some(directory.path().into()), true, "conversation-1".into())
                .await
                .unwrap()
                .conversation_id,
            "conversation-1"
        );
    }

    #[tokio::test]
    async fn request_fails_without_a_negotiated_capability() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(Some(directory.path().into()), true, "conversation-1".into())
                .await
                .unwrap_err()
                .to_string()
                .contains("upgrade gentd")
        );
    }

    #[tokio::test]
    async fn request_rejects_status_for_another_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec!["conversation-status-v1".into()]),
                }),
            )
            .await
            .unwrap();
            let _ = read_json_frame::<_, ConversationStatusFrame>(&mut stream)
                .await
                .unwrap();
            write_json_frame(
                &mut stream,
                &ConversationStatusFrame::Status(ConversationStatus {
                    conversation_id: "conversation-2".into(),
                    runs: Vec::new(),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(Some(directory.path().into()), true, "conversation-1".into())
                .await
                .unwrap_err()
                .to_string()
                .contains("conversation status")
        );
    }
}
