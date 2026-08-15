use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Event, HostEpoch};

/// Opaque, projection-owned state captured at a durable event cursor.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSnapshot {
    pub cursor: u64,
    pub host_epoch: HostEpoch,
    pub schema_version: u16,
    pub payload: Value,
}

/// A safe event-feed continuation. A resync replaces the client projection first.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventResume {
    Delta {
        events: Vec<Event>,
    },
    Resync {
        snapshot: EventSnapshot,
        events: Vec<Event>,
    },
}
