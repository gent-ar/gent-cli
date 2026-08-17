//! Protocol-only reader for authority-gated, content-free conversation activity.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame, Hello, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivity, PROTOCOL_MAX, PROTOCOL_MIN,
};
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
    if activity.schema_version != CONVERSATION_ACTIVITY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported conversation activity schema version {}",
            activity.schema_version
        ));
    }
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
#[path = "conversation_activity_tests.rs"]
mod tests;
