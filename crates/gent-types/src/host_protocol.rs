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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_digest_sha256: Option<String>,
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
