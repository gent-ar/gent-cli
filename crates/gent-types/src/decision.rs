//! Value types for durable, provider-neutral permission and question decisions.

use serde::{Deserialize, Serialize};

/// A client decision identified independently from the command receipt.
///
/// `idempotency_key` makes retried submissions distinguishable from a new decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionCommand {
    pub decision_id: String,
    pub idempotency_key: String,
}

/// The provider-observable portion of a decision's durable lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionSettlementPhase {
    Pending,
    Acknowledged,
    Settled,
    Unprovable,
    RecoveryRequired,
}

impl DecisionSettlementPhase {
    /// Returns whether no later provider evidence may change this decision.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Settled | Self::Unprovable | Self::RecoveryRequired
        )
    }
}

/// A decision together with the immutable idempotency key that created it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSettlement {
    pub decision_id: String,
    pub idempotency_key: String,
    pub phase: DecisionSettlementPhase,
}
