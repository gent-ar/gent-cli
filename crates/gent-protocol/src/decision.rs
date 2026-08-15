//! Wire-safe decision commands and outcomes; policy remains in `gent-core`.

use gent_types::DecisionSettlement;
use serde::{Deserialize, Serialize};

/// Evidence that may advance a durable decision after its initial submission.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionEvidence {
    ProviderAcknowledged,
    ProviderSettled,
    AcknowledgementUnprovable,
    RecoveryRequired,
}

/// Provider lifecycle evidence accepted only from daemon-owned runner or bridge code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderDecisionEvidence {
    ProviderAcknowledged,
    ProviderSettled,
}

/// Explicit terminal recovery evidence a local user may submit after a decision is pending.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionRecoveryEvidence {
    AcknowledgementUnprovable,
    RecoveryRequired,
}

/// A protocol representation of an idempotent decision submission outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", content = "decision", rename_all = "camelCase")]
pub enum DecisionSubmission {
    Accepted(DecisionSettlement),
    Duplicate(DecisionSettlement),
    IdempotencyConflict { existing_decision_id: String },
    DecisionIdConflict { existing_idempotency_key: String },
}

#[cfg(test)]
mod tests {
    use super::{DecisionEvidence, DecisionRecoveryEvidence, DecisionSubmission};
    use crate::WireFrame;
    use gent_types::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};

    #[test]
    fn frames_preserve_durable_outcomes_and_evidence() {
        let submitted = WireFrame::DecisionSubmit(DecisionCommand {
            decision_id: "allow-write".into(),
            idempotency_key: "receipt-42".into(),
        });
        let outcome =
            WireFrame::DecisionSubmission(DecisionSubmission::Accepted(DecisionSettlement {
                decision_id: "allow-write".into(),
                idempotency_key: "receipt-42".into(),
                phase: DecisionSettlementPhase::Pending,
            }));
        let evidence = WireFrame::DecisionEvidence {
            decision_id: "allow-write".into(),
            evidence: DecisionEvidence::AcknowledgementUnprovable,
        };
        let recovery = WireFrame::DecisionRecovery {
            decision_id: "allow-write".into(),
            evidence: DecisionRecoveryEvidence::AcknowledgementUnprovable,
        };
        for frame in [submitted, outcome, evidence, recovery] {
            let encoded = serde_json::to_vec(&frame).unwrap();
            assert_eq!(
                serde_json::from_slice::<WireFrame>(&encoded).unwrap(),
                frame
            );
        }
    }
}
