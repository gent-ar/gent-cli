//! Serializable run-scoped lifecycle snapshots shared across persistence and runtime boundaries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ConversationLiveStatus, HostEpoch, RootActivity, TurnPhase, WorkPhase};

/// The complete pure lifecycle state required to resume a run projection after a restart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLifecycleProjection {
    pub cursor: u64,
    pub active_turn_id: Option<String>,
    pub root_phase: TurnPhase,
    pub root_activity: RootActivity,
    pub children: BTreeMap<String, WorkPhase>,
    pub commands: BTreeMap<String, WorkPhase>,
    pub needs_attention: bool,
    pub has_error: bool,
}

impl Default for RunLifecycleProjection {
    fn default() -> Self {
        Self {
            cursor: 0,
            active_turn_id: None,
            root_phase: TurnPhase::Ready,
            root_activity: RootActivity::Idle,
            children: BTreeMap::new(),
            commands: BTreeMap::new(),
            needs_attention: false,
            has_error: false,
        }
    }
}

/// Durable projection data attributed to a specific host generation and immutable run lineage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProjectionRecord {
    pub run_id: String,
    pub host_epoch: HostEpoch,
    pub projection: RunLifecycleProjection,
}

/// Read-only live state for one run. It never includes provider-native session credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLiveStatus {
    pub run_id: String,
    pub host_epoch: HostEpoch,
    pub status: ConversationLiveStatus,
}
