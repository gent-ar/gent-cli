use super::decode::{delta, deltas, normalize_projection_transcript, readable_and_unreadable};
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

fn transcript_record(cursor: u64, payload: serde_json::Value) -> AgentChatProjectionEvent {
    AgentChatProjectionEvent {
        cursor,
        source_event_id: format!("source-{cursor}"),
        kind: "transcript".into(),
        payload,
    }
}

fn readable_transcript(cursor: u64) -> serde_json::Value {
    json!({
        "cursor": cursor, "eventId": format!("event-{cursor}"), "turnId": "turn-1",
        "runId": "run-1", "kind": "assistantMessage", "text": "kept", "isPartial": 0,
        "origin": null, "attachments": []
    })
}

#[test]
fn an_unreadable_record_is_one_typed_fact_and_the_records_after_it_are_still_served() {
    let unreadable = |cursor: u64, kind: &str| AgentChatProjectionEvent {
        cursor,
        source_event_id: format!("source-{cursor}"),
        kind: kind.into(),
        payload: json!({"lifecycle": "not a lifecycle fact"}),
    };
    let served = deltas(
        vec![
            unreadable(4, "lifecycle"),
            transcript_record(5, readable_transcript(5)),
            unreadable(6, "retired"),
        ],
        &no_summary,
    )
    .unwrap();
    assert_eq!(
        served
            .iter()
            .map(|delta| match delta {
                AgentChatProjectionDelta::UnreadableRecord {
                    cursor,
                    record_kind,
                } => format!("{}:{record_kind:?}", cursor.value),
                AgentChatProjectionDelta::Transcript { cursor, event } => {
                    format!("{}:{}", cursor.value, event.text)
                }
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        ["4:Lifecycle", "5:kept", "6:Unrecognized"]
    );
}

#[test]
fn a_summary_that_cannot_be_read_fails_the_follow_instead_of_marking_the_record() {
    let summary = AgentChatProjectionEvent {
        cursor: 7,
        source_event_id: "summary:7".into(),
        kind: "summary".into(),
        payload: json!({"runId": "run-1"}),
    };
    assert_eq!(
        deltas(vec![summary], &no_summary),
        Err("no summary in this fixture".to_owned())
    );
}

#[test]
fn a_snapshot_window_keeps_its_readable_records_and_lists_the_unreadable_ones() {
    let (readable, unreadable) = readable_and_unreadable(
        vec![
            transcript_record(2, readable_transcript(2)),
            transcript_record(3, json!({"kind": "retiredShape"})),
        ],
        gent_protocol::ProjectionRecordKind::Transcript,
        super::decode::transcript_event,
    );
    assert_eq!(
        readable
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        ["event-2"]
    );
    assert_eq!(
        unreadable,
        [gent_protocol::UnreadableProjectionRecord {
            cursor: gent_protocol::ProjectionCursor { value: 3 },
            record_kind: gent_protocol::ProjectionRecordKind::Transcript,
        }]
    );
}
