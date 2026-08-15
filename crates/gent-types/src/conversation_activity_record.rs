//! Durable internal state required to resume a conversation-activity reducer.

use serde::{Deserialize, Serialize};

use crate::{ConversationActivity, ConversationActivityState, RootActivity};

/// Complete restart-safe state for the pure conversation-activity state machine.
///
/// This record is an internal persistence DTO, not a client-facing protocol value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityRecord {
    pub activity: ConversationActivity,
    pub root_activity: RootActivity,
    pub terminal_turn_ids: Vec<String>,
    pub transition: Option<ConversationActivityState>,
}
