use gent_protocol::{
    AgentChatConversationFrame, AgentChatProjectionDelta, AgentChatProjectionFrame,
    AgentChatProjectionSnapshot, AgentChatTranscriptFrame, ProjectionCursor, ProjectionRecordKind,
};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;
use decode::{activity_fact, deltas, readable_and_unreadable, transcript_event};

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
        return window::read(facade, frame);
    };
    let reads = projection_reads(facade)?;
    let mut detail = reads
        .detail(&conversation_id)
        .map_err(AgentChatIntentError::from)?;
    detail.summary = summary_with_mcp(facade, detail.summary);
    let tail = reads
        .projection_tail(&conversation_id, transcript_limit, activity_limit)
        .map_err(AgentChatIntentError::from)?;
    let (transcript, mut unreadable) = readable_and_unreadable(
        tail.transcript,
        ProjectionRecordKind::Transcript,
        transcript_event,
    );
    let (activity, unreadable_activity) =
        readable_and_unreadable(tail.activity, ProjectionRecordKind::Activity, activity_fact);
    unreadable.extend(unreadable_activity);
    unreadable.sort_by_key(|record| record.cursor.value);
    Ok(AgentChatProjectionFrame::ConversationSnapshot {
        request_id,
        snapshot: Box::new(AgentChatProjectionSnapshot {
            conversation: detail,
            cursor: ProjectionCursor { value: tail.cursor },
            catalogs: Vec::new(),
            transcript,
            activity,
            unreadable,
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
    deltas(events, &summary)
}

fn projection_reads(
    facade: &RuntimeFacade,
) -> Result<&gent_runtime::AgentChatReadService<gent_store::SqliteLedger>, String> {
    facade
        .agent_chat_reads
        .as_ref()
        .ok_or_else(|| "agent-chat projection is observer-disabled".to_owned())
}

#[path = "runtime_facade_projection_decode.rs"]
mod decode;

#[path = "runtime_facade_projection_window.rs"]
mod window;

#[cfg(test)]
#[path = "runtime_facade_chat_reads_tests.rs"]
mod tests;
