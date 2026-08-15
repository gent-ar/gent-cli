//! Protocol-only client request for a durable, non-content conversation timeline.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_TIMELINE_CAPABILITY, ConversationTimelineFrame, Hello, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{ConversationTimeline, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::Value;

use crate::local_ipc::{client_capabilities, connect_or_start};

/// Reads durable lineage and lifecycle metadata without creating a receipt or exposing content.
///
/// # Errors
/// Returns an error when the daemon is unavailable, unnegotiated, or lacks the capability.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<ConversationTimeline, Box<dyn std::error::Error>> {
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
        .any(|capability| capability == CONVERSATION_TIMELINE_CAPABILITY)
    {
        return Err("daemon does not support conversation timeline; upgrade gentd".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationTimelineFrame::TimelineRequest { conversation_id },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationTimelineFrame::Timeline(timeline)) = serde_json::from_value(raw.clone()) {
        return Ok(timeline);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation timeline".into())
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{
        CONVERSATION_TIMELINE_CAPABILITY, ConversationTimelineFrame, Hello, Negotiated, read_frame,
        read_json_frame, write_frame, write_json_frame,
    };
    use gent_types::{CapabilitySet, ConversationTimeline, PROTOCOL_MAX};
    use tokio::net::UnixListener;

    use super::request;

    #[tokio::test]
    async fn request_requires_capability_and_returns_a_non_content_timeline() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                gent_protocol::WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == CONVERSATION_TIMELINE_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![CONVERSATION_TIMELINE_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, ConversationTimelineFrame>(&mut stream)
                    .await
                    .unwrap(),
                ConversationTimelineFrame::TimelineRequest { conversation_id }
                    if conversation_id == "conversation-1"
            ));
            write_json_frame(
                &mut stream,
                &ConversationTimelineFrame::Timeline(ConversationTimeline {
                    conversation_id: "conversation-1".into(),
                    runs: Vec::new(),
                    artifacts: Vec::new(),
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
}
