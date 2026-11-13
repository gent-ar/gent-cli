use super::{delta, deltas_until_unreadable, normalize_projection_transcript};
use gent_protocol::AgentChatProjectionDelta;
use gent_types::{AgentChatProjectionEvent, AttachmentReference};
use serde_json::json;

fn no_summary() -> Result<gent_types::AgentChatConversationSummary, String> {
    Err("no summary in this fixture".into())
}

#[test]
fn projected_user_messages_keep_their_attachment_references() {
    let projected = delta(
        AgentChatProjectionEvent {
        cursor: 9,
        source_event_id: "transcript:user:message-1".into(),
        kind: "transcript".into(),
        payload: json!({
            "cursor": 3, "eventId": "user:message-1", "turnId": "turn-1", "runId": "run-1",
            "kind": "userMessage", "text": "look", "isPartial": 0, "origin": null,
            "attachments": [{"attachmentId": "attachment-1", "displayName": "shot.png", "mediaType": "image/png", "byteLen": 5}],
        }),
        },
        &no_summary,
    )
    .unwrap();
    let AgentChatProjectionDelta::Transcript { event, .. } = projected else {
        panic!("expected a transcript delta");
    };
    assert_eq!(
        event.attachments,
        vec![AttachmentReference {
            attachment_id: "attachment-1".into(),
            display_name: "shot.png".into(),
            media_type: "image/png".into(),
            byte_len: 5,
        }]
    );
}

#[test]
fn normalizes_sqlite_json_boolean_for_transcript_replay() {
    assert_eq!(
        normalize_projection_transcript(json!({"isPartial": 0})),
        json!({"isPartial": false})
    );
    assert_eq!(
        normalize_projection_transcript(json!({"isPartial": 1})),
        json!({"isPartial": true})
    );
}

#[test]
fn an_unreadable_event_ends_the_batch_with_a_resync_at_its_cursor() {
    let event = |cursor: u64, kind: &str| AgentChatProjectionEvent {
        cursor,
        source_event_id: format!("source-{cursor}"),
        kind: kind.into(),
        payload: json!({"lifecycle": "not a lifecycle fact"}),
    };
    assert_eq!(
        deltas_until_unreadable(
            vec![event(4, "lifecycle"), event(5, "lifecycle")],
            &no_summary
        ),
        [AgentChatProjectionDelta::ResyncRequired {
            cursor: gent_protocol::ProjectionCursor { value: 4 },
        }]
    );
}
