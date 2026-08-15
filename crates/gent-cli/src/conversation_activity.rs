//! Protocol-only reader for authority-gated, content-free conversation activity.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame, Hello, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{ConversationActivity, PROTOCOL_MAX, PROTOCOL_MIN};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::local_ipc::{client_capabilities, connect_or_start};

/// Content-free response that callers can use to replace or advance local activity state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ActivityRead {
    Snapshot(ConversationActivity),
    Delta(Vec<ConversationActivity>),
}

/// Reads a snapshot or cursor-ordered activity delta without creating a receipt.
///
/// # Errors
/// Returns an error when the daemon is unavailable, unnegotiated, or not authoritative.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
    after_cursor: u64,
) -> Result<ActivityRead, Box<dyn std::error::Error>> {
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
        .any(|capability| capability == CONVERSATION_ACTIVITY_CAPABILITY)
    {
        return Err("daemon does not support authoritative conversation activity".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationActivityFrame::Request {
            conversation_id,
            run_id,
            after_cursor,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationActivityFrame::Snapshot(activity)) = serde_json::from_value(raw.clone()) {
        return Ok(ActivityRead::Snapshot(activity));
    }
    if let Ok(ConversationActivityFrame::Delta(activities)) = serde_json::from_value(raw.clone()) {
        return Ok(ActivityRead::Delta(activities));
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation activity".into())
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Negotiated, read_frame, read_json_frame, write_frame, write_json_frame};
    use gent_types::{
        CONVERSATION_ACTIVITY_SCHEMA_VERSION, CapabilitySet, ConversationActivity,
        ConversationActivityState, HostEpoch, TurnPhase,
    };
    use tokio::net::UnixListener;

    use super::*;

    fn activity() -> ConversationActivity {
        ConversationActivity {
            schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            host_epoch: HostEpoch(1),
            revision: 2,
            activity_sequence: 2,
            cursor: 9,
            active_turn_id: Some("turn-1".into()),
            root_phase: TurnPhase::Processing,
            state: ConversationActivityState::Thinking,
            pending_decision_id: None,
            work: Vec::new(),
            has_error: false,
        }
    }

    #[tokio::test]
    async fn request_negotiates_and_reads_a_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == CONVERSATION_ACTIVITY_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![CONVERSATION_ACTIVITY_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, ConversationActivityFrame>(&mut stream).await.unwrap(),
                ConversationActivityFrame::Request { conversation_id, run_id, after_cursor }
                    if conversation_id == "conversation-1" && run_id == "run-1" && after_cursor == 4
            ));
            write_json_frame(
                &mut stream,
                &ConversationActivityFrame::Snapshot(activity()),
            )
            .await
            .unwrap();
        });
        assert!(matches!(
            request(Some(directory.path().into()), true, "conversation-1".into(), "run-1".into(), 4)
                .await
                .unwrap(),
            ActivityRead::Snapshot(activity) if activity.cursor == 9
        ));
    }

    #[tokio::test]
    async fn observer_capability_set_rejects_the_authority_gated_read() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(Vec::new()),
                }),
            )
            .await
            .unwrap();
        });
        let error = request(
            Some(directory.path().into()),
            true,
            "conversation-1".into(),
            "run-1".into(),
            0,
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("authoritative conversation activity")
        );
    }
}
