use serde::{Deserialize, Serialize};

use crate::Event;

/// A bounded, cursor-ordered page from the immutable durable event log.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub events: Vec<Event>,
    /// Cursor to use for the next page, when more retained events exist.
    pub next_after_cursor: Option<u64>,
}
