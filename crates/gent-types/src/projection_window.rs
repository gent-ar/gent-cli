use serde::{Deserialize, Serialize};

use crate::{AgentChatProjectionEvent, TurnRecord};

pub const MAX_TURN_CURSOR_PAGE: u16 = 200;
pub const MAX_PROJECTION_WINDOW_CARRY: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationTurnCursor {
    pub turn_id: String,
    pub run_id: String,
    pub first_projection_cursor: u64,
    pub first_transcript_cursor: Option<u64>,
    pub first_activity_cursor: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationTurnCursorPage {
    pub turns: Vec<ConversationTurnCursor>,
    pub next_before_cursor: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWindowCarry {
    pub last_turn: Option<TurnRecord>,
    pub events: Vec<AgentChatProjectionEvent>,
}
