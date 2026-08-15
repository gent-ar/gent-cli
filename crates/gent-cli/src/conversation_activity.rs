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
            conversation_id: conversation_id.clone(),
            run_id: run_id.clone(),
            after_cursor,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(frame) = serde_json::from_value(raw.clone()) {
        return decode(frame, &conversation_id, &run_id, after_cursor).map_err(Into::into);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation activity".into())
}

fn decode(
    frame: ConversationActivityFrame,
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
) -> Result<ActivityRead, String> {
    match frame {
        ConversationActivityFrame::Snapshot(activity) => {
            validate_activity(&activity, conversation_id, run_id)?;
            if activity.cursor < after_cursor {
                return Err("daemon returned a stale activity snapshot".into());
            }
            Ok(ActivityRead::Snapshot(activity))
        }
        ConversationActivityFrame::Delta(activities) => {
            validate_delta(&activities, conversation_id, run_id, after_cursor)?;
            Ok(ActivityRead::Delta(activities))
        }
        ConversationActivityFrame::Request { .. } => {
            Err("daemon returned an activity request instead of a response".into())
        }
    }
}

fn validate_delta(
    activities: &[ConversationActivity],
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
) -> Result<(), String> {
    let mut cursor = after_cursor;
    let mut previous: Option<&ConversationActivity> = None;
    for activity in activities {
        validate_activity(activity, conversation_id, run_id)?;
        if activity.cursor <= cursor {
            return Err("daemon returned non-monotonic activity cursors".into());
        }
        if let Some(previous) = previous {
            if activity.revision <= previous.revision
                || activity.activity_sequence <= previous.activity_sequence
                || activity.host_epoch != previous.host_epoch
            {
                return Err("daemon returned an inconsistent activity delta".into());
            }
        }
        cursor = activity.cursor;
        previous = Some(activity);
    }
    Ok(())
}

fn validate_activity(
    activity: &ConversationActivity,
    conversation_id: &str,
    run_id: &str,
) -> Result<(), String> {
    if activity.conversation_id != conversation_id || activity.run_id != run_id {
        return Err("daemon returned activity for another conversation run".into());
    }
    Ok(())
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

    #[test]
    fn delta_requires_one_identity_and_strictly_increasing_activity_state() {
        let first = activity();
        let mut second = activity();
        second.cursor = 10;
        second.revision = 3;
        second.activity_sequence = 3;
        assert!(matches!(
            decode(
                ConversationActivityFrame::Delta(vec![first.clone(), second]),
                "conversation-1",
                "run-1",
                8,
            ),
            Ok(ActivityRead::Delta(items)) if items.len() == 2
        ));

        assert!(
            decode(
                ConversationActivityFrame::Delta(vec![first, activity()]),
                "conversation-1",
                "run-1",
                8,
            )
            .is_err()
        );

        let mut wrong_run = activity();
        wrong_run.run_id = "other".into();
        assert!(
            decode(
                ConversationActivityFrame::Delta(vec![wrong_run]),
                "conversation-1",
                "run-1",
                8,
            )
            .is_err()
        );
    }

    #[test]
    fn snapshot_cannot_regress_the_client_cursor() {
        assert!(
            decode(
                ConversationActivityFrame::Snapshot(activity()),
                "conversation-1",
                "run-1",
                10,
            )
            .is_err()
        );
    }
}
