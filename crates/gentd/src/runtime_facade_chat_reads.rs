use gent_protocol::{
    AgentChatConversationFrame, AgentChatProjectionDelta, AgentChatProjectionFrame,
    AgentChatProjectionSnapshot, AgentChatTranscriptFrame, ProjectionCursor,
};
use gent_types::{
    AgentChatConversationSummary, AgentChatFailureCode, AgentChatProjectionEvent,
    ConversationActivityFact, NormalizedSessionLifecycle, NormalizedTranscriptEvent,
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
            .map(|summary| AgentChatConversationFrame::Summary(summary_with_mcp(facade, summary))),
        AgentChatConversationFrame::DetailRequest { conversation_id } => reads
            .detail(&conversation_id)
            .map_err(AgentChatIntentError::from)
            .map(|mut detail| {
                detail.summary = summary_with_mcp(facade, detail.summary);
                AgentChatConversationFrame::Detail(detail)
            }),
        AgentChatConversationFrame::Summary(_) | AgentChatConversationFrame::Detail(_) => {
            Err("agent-chat conversation response frames are server-only".into())
        }
    }
}

fn summary_with_mcp(
    facade: &RuntimeFacade,
    mut summary: gent_types::AgentChatConversationSummary,
) -> gent_types::AgentChatConversationSummary {
    summary.mcp_server_count = facade.mcp_server_count;
    summary
        .mcp_server_names
        .clone_from(&facade.mcp_server_names);
    summary
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
    detail.summary = summary_with_mcp(facade, detail.summary);
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
    let reads = projection_reads(facade)?;
    let events = reads
        .projection(conversation_id, after_cursor.value, 100)
        .map_err(|error| error.to_string())?
        .events;
    let summary = || {
        reads
            .summary(conversation_id)
            .map(|summary| summary_with_mcp(facade, summary))
            .map_err(|error| error.to_string())
    };
    Ok(deltas_until_unreadable(events, &summary))
}

type SummaryRead<'a> = &'a dyn Fn() -> Result<AgentChatConversationSummary, String>;

fn deltas_until_unreadable(
    events: Vec<AgentChatProjectionEvent>,
    summary: SummaryRead<'_>,
) -> Vec<AgentChatProjectionDelta> {
    let mut deltas = Vec::new();
    for event in events {
        let cursor = ProjectionCursor {
            value: event.cursor,
        };
        match delta(event, summary) {
            Ok(delta) => deltas.push(delta),
            Err(error) => {
                eprintln!(
                    "projection event {} requires a resync: {error}",
                    cursor.value
                );
                deltas.push(AgentChatProjectionDelta::ResyncRequired { cursor });
                break;
            }
        }
    }
    deltas
}

fn projection_reads(
    facade: &RuntimeFacade,
) -> Result<&gent_runtime::AgentChatReadService<gent_store::SqliteLedger>, String> {
    facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat projection is observer-disabled".to_owned())
}

fn delta(
    event: AgentChatProjectionEvent,
    summary: SummaryRead<'_>,
) -> Result<AgentChatProjectionDelta, String> {
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
        "summary" => Ok(AgentChatProjectionDelta::Summary {
            cursor,
            run_id: event
                .payload
                .get("runId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "projection summary has no run".to_owned())?
                .to_owned(),
            summary: Box::new(summary()?),
        }),
        other => Err(format!("unknown projection event kind {other}")),
    }
}

fn transcript_event(payload: serde_json::Value) -> Result<NormalizedTranscriptEvent, String> {
    let mut event: NormalizedTranscriptEvent =
        serde_json::from_value(normalize_projection_transcript(payload))
            .map_err(|error| error.to_string())?;
    event.failure_code = AgentChatFailureCode::from_event_id(&event.event_id);
    Ok(event)
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
#[path = "runtime_facade_chat_reads_tests.rs"]
mod tests;
