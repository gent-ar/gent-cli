//! Stable value types shared by every public Gent crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const PROTOCOL_MIN: u16 = 1;
pub const PROTOCOL_MAX: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HostEpoch(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);

impl ReceiptId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for ReceiptId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptStatus {
    Accepted,
    Settled,
    Unprovable,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub status: ReceiptStatus,
    pub host_epoch: HostEpoch,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub cursor: u64,
    pub event_id: String,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub kind: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitySet(pub Vec<String>);

impl CapabilitySet {
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let mut shared = self
            .0
            .iter()
            .filter(|capability| other.0.contains(*capability))
            .cloned()
            .collect::<Vec<_>>();
        shared.sort();
        shared.dedup();
        Self(shared)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_epoch: HostEpoch,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub name: String,
    pub present: bool,
    pub version: Option<String>,
    pub remediation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub dependencies: Vec<DependencyStatus>,
}

/// Immutable provenance captured before a public-provider process is allowed to start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunVersionLock {
    pub provider: String,
    pub canonical_path: String,
    pub file_identity: String,
    pub digest_sha256: String,
    pub version: String,
    pub compatibility_entry: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEvent {
    Output { text: String },
    DecisionSettled { decision_id: String },
    Terminal { reason: String },
}

/// Durable root-turn state. Detached work never changes this state to completed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnPhase {
    Processing,
    WaitingPermission,
    WaitingQuestion,
    Compacting,
    Ready,
    Interrupted,
    Dead,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkPhase {
    Pending,
    Running,
    WaitingPermission,
    Done,
    Failed,
    Interrupted,
}

impl WorkPhase {
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Running | Self::WaitingPermission
        )
    }
}

/// A complete volatile snapshot sent over status transport, never transcript content.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // The wire contract intentionally transports independent UI facts.
pub struct ConversationLiveStatus {
    pub snapshot_cursor: u64,
    pub is_processing: bool,
    pub is_waiting_for_subagents: bool,
    pub has_live_subagent_work: bool,
    pub is_waiting_for_command: bool,
    pub has_live_command_work: bool,
    pub needs_attention: bool,
    pub has_error: bool,
}

#[cfg(test)]
mod tests {
    use super::CapabilitySet;

    #[test]
    fn capability_intersection_is_sorted_and_unique() {
        let left = CapabilitySet(vec!["events".into(), "receipts".into(), "events".into()]);
        let right = CapabilitySet(vec!["events".into(), "status".into()]);
        assert_eq!(
            left.intersection(&right),
            CapabilitySet(vec!["events".into()])
        );
    }
}
