use gent_protocol::{AgentChatProjectionFrame, ProjectionCursor};

use super::{RuntimeFacade, deltas, projection_reads, summary_with_mcp};
use crate::agent_chat_intent_error::AgentChatIntentError;

pub(super) fn read(
    facade: &RuntimeFacade,
    frame: AgentChatProjectionFrame,
) -> Result<AgentChatProjectionFrame, AgentChatIntentError> {
    let reads = projection_reads(facade)?;
    match frame {
        AgentChatProjectionFrame::ConversationTurnCursorsRequest {
            request_id,
            conversation_id,
            before_cursor,
            limit,
        } => {
            reads
                .summary(&conversation_id)
                .map_err(AgentChatIntentError::from)?;
            if limit == 0 || limit > gent_types::MAX_TURN_CURSOR_PAGE {
                return Err(AgentChatIntentError::coded(
                    "turnCursorLimitInvalid",
                    "the turn cursor page limit is out of range",
                ));
            }
            let page = facade
                .transcript_import_ledger
                .conversation_turn_cursors(
                    &conversation_id,
                    before_cursor.map(|cursor| cursor.value),
                    limit,
                )
                .map_err(|error| error.to_string())?;
            Ok(AgentChatProjectionFrame::ConversationTurnCursors {
                request_id,
                conversation_id,
                turns: page.turns,
                next_before_cursor: page
                    .next_before_cursor
                    .map(|value| ProjectionCursor { value }),
            })
        }
        AgentChatProjectionFrame::ConversationWindowRequest {
            request_id,
            conversation_id,
            from_cursor,
        } => {
            let summary = || {
                reads
                    .summary(&conversation_id)
                    .map(|summary| summary_with_mcp(facade, summary))
                    .map_err(|error| error.to_string())
            };
            summary().map_err(AgentChatIntentError::from)?;
            let carry = facade
                .transcript_import_ledger
                .projection_window_carry(&conversation_id, from_cursor.value)
                .map_err(|error| match error {
                    gent_ports::LedgerError::Invariant(message) => {
                        AgentChatIntentError::coded("projectionWindowTooLarge", &message)
                    }
                    error => error.to_string().into(),
                })?;
            let carried = deltas(carry.events, &summary)?;
            Ok(AgentChatProjectionFrame::ConversationWindow {
                request_id,
                conversation_id,
                from_cursor,
                last_turn: carry.last_turn,
                carried,
            })
        }
        _ => Err("agent-chat projection accepts only read requests here".into()),
    }
}
