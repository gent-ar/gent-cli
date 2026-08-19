//! Immutable, cursor-ordered lifecycle facts for one run.

use serde::{Deserialize, Serialize};

use crate::{ConversationLiveStatus, HostEpoch, NormalizedSessionLifecycle};

/// One daemon-normalized lifecycle fact committed with its source event cursor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLifecycleFact {
    pub run_id: String,
    pub event_id: String,
    pub host_epoch: HostEpoch,
    pub cursor: u64,
    pub lifecycle: NormalizedSessionLifecycle,
}

/// A bounded, strictly cursor-ordered page of immutable lifecycle facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLifecycleFactPage {
    pub facts: Vec<RunLifecycleFact>,
    pub next_after_cursor: Option<u64>,
}

/// Read-only live state derived by replaying immutable lifecycle facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLiveStatus {
    pub run_id: String,
    pub host_epoch: HostEpoch,
    pub status: ConversationLiveStatus,
}
