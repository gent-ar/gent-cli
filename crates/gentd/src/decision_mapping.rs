//! Pure conversion between core decision outcomes and protocol DTOs.

use gent_core::{DecisionCommandOutcome, DecisionEvidence as CoreDecisionEvidence};
use gent_protocol::{DecisionRecoveryEvidence, DecisionSubmission};

pub(super) fn submission(outcome: DecisionCommandOutcome) -> DecisionSubmission {
    match outcome {
        DecisionCommandOutcome::Accepted(decision) => DecisionSubmission::Accepted(decision),
        DecisionCommandOutcome::Duplicate(decision) => DecisionSubmission::Duplicate(decision),
        DecisionCommandOutcome::IdempotencyConflict {
            existing_decision_id,
        } => DecisionSubmission::IdempotencyConflict {
            existing_decision_id,
        },
        DecisionCommandOutcome::DecisionIdConflict {
            existing_idempotency_key,
        } => DecisionSubmission::DecisionIdConflict {
            existing_idempotency_key,
        },
    }
}

pub(super) const fn recovery(evidence: DecisionRecoveryEvidence) -> CoreDecisionEvidence {
    match evidence {
        DecisionRecoveryEvidence::AcknowledgementUnprovable => {
            CoreDecisionEvidence::AcknowledgementUnprovable
        }
        DecisionRecoveryEvidence::RecoveryRequired => CoreDecisionEvidence::RecoveryRequired,
    }
}
