//! Provider-neutral chat read helpers kept apart from the runtime API implementation.

use gent_protocol::{
    AgentChatConversationFrame, AgentChatProjectionDelta, AgentChatProjectionFrame,
    AgentChatProjectionSnapshot, AgentChatTranscriptFrame, ProjectionCursor,
};
use gent_types::{
    AgentChatProjectionEvent, ConversationActivityFact, NormalizedSessionLifecycle,
    NormalizedTranscriptEvent,
};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;

pub(super) fn conversation(
    facade: &RuntimeFacade,
    frame: AgentChatConversationFrame,
) -> Result<AgentChatConversationFrame, AgentChatIntentError> {
    let reads = facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat conversation reads are observer-disabled".to_owned())?;
    match frame {
        AgentChatConversationFrame::SummaryRequest { conversation_id } => reads
            .summary(&conversation_id)
            .map_err(AgentChatIntentError::from)
            .and_then(|summary| {
                summary_with_mcp(facade, summary)
                    .map(AgentChatConversationFrame::Summary)
                    .map_err(AgentChatIntentError::from)
            }),
        AgentChatConversationFrame::DetailRequest { conversation_id } => reads
            .detail(&conversation_id)
            .map_err(AgentChatIntentError::from)
            .and_then(|mut detail| {
                detail.summary = summary_with_mcp(facade, detail.summary)?;
                Ok(AgentChatConversationFrame::Detail(detail))
            }),
        AgentChatConversationFrame::Summary(_) | AgentChatConversationFrame::Detail(_) => {
            Err("agent-chat conversation response frames are server-only".into())
        }
    }
}

fn summary_with_mcp(
    facade: &RuntimeFacade,
    mut summary: gent_types::AgentChatConversationSummary,
) -> Result<gent_types::AgentChatConversationSummary, String> {
    summary.mcp_server_count = facade.mcp_server_count;
    summary
        .mcp_server_names
        .clone_from(&facade.mcp_server_names);
    let git = match summary.workspace_path.as_deref() {
        Some(path) => crate::workspace_git_api::status(path)?,
        None => None,
    };
    summary.changed_file_count = git
        .as_ref()
        .map(|report| u32::try_from(report.files.len()).unwrap_or(u32::MAX));
    summary.git_branch = git.and_then(|report| report.branch);
    Ok(summary)
}

pub(super) fn transcript(
    facade: &RuntimeFacade,
    frame: AgentChatTranscriptFrame,
) -> Result<AgentChatTranscriptFrame, AgentChatIntentError> {
    let reads = facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat transcript reads are observer-disabled".to_owned())?;
    match frame {
        AgentChatTranscriptFrame::PageRequest {
            conversation_id,
            after_cursor,
            limit,
        } => reads
            .transcript(&conversation_id, after_cursor, limit)
            .map(AgentChatTranscriptFrame::Page)
            .map_err(AgentChatIntentError::from),
        AgentChatTranscriptFrame::Page(_) => {
            Err("agent-chat transcript response frames are server-only".into())
        }
    }
}

pub(super) fn projection(
    facade: &RuntimeFacade,
    frame: AgentChatProjectionFrame,
) -> Result<AgentChatProjectionFrame, AgentChatIntentError> {
    let AgentChatProjectionFrame::ConversationSnapshotRequest {
        request_id,
        conversation_id,
        transcript_limit,
        activity_limit,
    } = frame
    else {
        return Err("agent-chat projection accepts only snapshot requests here".into());
    };
    let reads = projection_reads(facade)?;
    let mut detail = reads
        .detail(&conversation_id)
        .map_err(AgentChatIntentError::from)?;
    detail.summary = summary_with_mcp(facade, detail.summary)?;
    let tail = reads
        .projection_tail(&conversation_id, transcript_limit, activity_limit)
        .map_err(AgentChatIntentError::from)?;
    Ok(AgentChatProjectionFrame::ConversationSnapshot {
        request_id,
        snapshot: Box::new(AgentChatProjectionSnapshot {
            conversation: detail,
            cursor: ProjectionCursor { value: tail.cursor },
            catalogs: Vec::new(),
            transcript: tail
                .transcript
                .into_iter()
                .map(|event| transcript_event(event.payload))
                .collect::<Result<_, _>>()?,
            activity: tail
                .activity
                .into_iter()
                .map(|event| activity_fact(event.payload))
                .collect::<Result<_, _>>()?,
            goal: match facade
                .goals
                .current(&gent_types::AgentChatConversationId(conversation_id))
                .map_err(|error| error.to_string())?
            {
                gent_runtime::GoalResult::Goal(goal) => goal,
                _ => None,
            },
            transcript_truncated: tail.transcript_truncated,
            activity_truncated: tail.activity_truncated,
        }),
    })
}

pub(super) fn follow(
    facade: &RuntimeFacade,
    conversation_id: &str,
    after_cursor: &ProjectionCursor,
) -> Result<Vec<AgentChatProjectionDelta>, String> {
    projection_reads(facade)?
        .projection(conversation_id, after_cursor.value, 100)
        .map_err(|error| error.to_string())?
        .events
        .into_iter()
        .map(delta)
        .collect()
}

fn projection_reads(
    facade: &RuntimeFacade,
) -> Result<&gent_runtime::AgentChatReadService<gent_store::SqliteLedger>, String> {
    facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat projection is observer-disabled".to_owned())
}

fn delta(event: AgentChatProjectionEvent) -> Result<AgentChatProjectionDelta, String> {
    let cursor = ProjectionCursor {
        value: event.cursor,
    };
    match event.kind.as_str() {
        "transcript" => Ok(AgentChatProjectionDelta::Transcript {
            cursor,
            event: Box::new(transcript_event(event.payload)?),
        }),
        "activity" => Ok(AgentChatProjectionDelta::Activity {
            cursor,
            fact: Box::new(activity_fact(event.payload)?),
        }),
        "lifecycle" => Ok(AgentChatProjectionDelta::Lifecycle {
            cursor,
            lifecycle: serde_json::from_value::<NormalizedSessionLifecycle>(
                event
                    .payload
                    .get("lifecycle")
                    .cloned()
                    .ok_or_else(|| "projection lifecycle has no fact".to_owned())?,
            )
            .map_err(|error| error.to_string())?,
        }),
        other => Err(format!("unknown projection event kind {other}")),
    }
}

fn transcript_event(payload: serde_json::Value) -> Result<NormalizedTranscriptEvent, String> {
    serde_json::from_value(normalize_projection_transcript(payload))
        .map_err(|error| error.to_string())
}

fn activity_fact(payload: serde_json::Value) -> Result<ConversationActivityFact, String> {
    serde_json::from_value(
        payload
            .get("activity")
            .cloned()
            .ok_or_else(|| "projection activity has no fact".to_owned())?,
    )
    .map_err(|error| error.to_string())
}

fn normalize_projection_transcript(mut payload: serde_json::Value) -> serde_json::Value {
    if let Some(partial) = payload.get_mut("isPartial") {
        if let Some(value) = partial.as_i64() {
            *partial = serde_json::Value::Bool(value != 0);
        }
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::{delta, normalize_projection_transcript};
    use gent_protocol::AgentChatProjectionDelta;
    use gent_types::{AgentChatProjectionEvent, AttachmentReference};
    use serde_json::json;

    #[test]
    fn projected_user_messages_keep_their_attachment_references() {
        let projected = delta(AgentChatProjectionEvent {
            cursor: 9,
            source_event_id: "transcript:user:message-1".into(),
            kind: "transcript".into(),
            payload: json!({
                "cursor": 3, "eventId": "user:message-1", "turnId": "turn-1", "runId": "run-1",
                "kind": "userMessage", "text": "look", "isPartial": 0, "origin": null,
                "attachments": [{"attachmentId": "attachment-1", "displayName": "shot.png", "mediaType": "image/png", "byteLen": 5}],
            }),
        })
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
}
