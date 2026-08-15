//! Pure, idempotent decision settlement policy.

use std::collections::BTreeMap;

use gent_types::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};

/// Decisions indexed by both their public ID and immutable idempotency key.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DecisionSettlementState {
    decisions: BTreeMap<String, DecisionSettlement>,
    keys: BTreeMap<String, String>,
}

impl DecisionSettlementState {
    /// Looks up a decision without exposing mutable storage.
    #[must_use]
    pub fn decision(&self, decision_id: &str) -> Option<&DecisionSettlement> {
        self.decisions.get(decision_id)
    }
}

/// The result of accepting or deduplicating a client decision command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionCommandOutcome {
    Accepted(DecisionSettlement),
    Duplicate(DecisionSettlement),
    IdempotencyConflict { existing_decision_id: String },
    DecisionIdConflict { existing_idempotency_key: String },
}

/// Successor state for a client decision command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionCommandUpdate {
    pub state: DecisionSettlementState,
    pub outcome: DecisionCommandOutcome,
}

/// Provider or recovery evidence that can advance a pending decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionEvidence {
    ProviderAcknowledged,
    ProviderSettled,
    AcknowledgementUnprovable,
    RecoveryRequired,
}

/// Successor state for provider or recovery evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionEvidenceUpdate {
    pub state: DecisionSettlementState,
    pub changed: bool,
}

/// Accepts one decision exactly once per idempotency key.
///
/// Retrying the same `(decision_id, idempotency_key)` pair returns its original state. Reusing
/// either identifier for a different command is rejected without mutating state.
#[must_use]
pub fn submit_decision(
    mut state: DecisionSettlementState,
    command: DecisionCommand,
) -> DecisionCommandUpdate {
    if let Some(existing_id) = state.keys.get(&command.idempotency_key) {
        let existing = state.decisions[existing_id].clone();
        let outcome = if existing.decision_id == command.decision_id {
            DecisionCommandOutcome::Duplicate(existing)
        } else {
            DecisionCommandOutcome::IdempotencyConflict {
                existing_decision_id: existing.decision_id,
            }
        };
        return DecisionCommandUpdate { state, outcome };
    }

    if let Some(existing) = state.decisions.get(&command.decision_id) {
        let existing_idempotency_key = existing.idempotency_key.clone();
        return DecisionCommandUpdate {
            state,
            outcome: DecisionCommandOutcome::DecisionIdConflict {
                existing_idempotency_key,
            },
        };
    }

    let decision = DecisionSettlement {
        decision_id: command.decision_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        phase: DecisionSettlementPhase::Pending,
    };
    state
        .keys
        .insert(command.idempotency_key, command.decision_id.clone());
    state
        .decisions
        .insert(command.decision_id, decision.clone());
    DecisionCommandUpdate {
        state,
        outcome: DecisionCommandOutcome::Accepted(decision),
    }
}

/// Applies provider or recovery evidence without clock, process, or persistence access.
///
/// `AcknowledgementUnprovable` and `RecoveryRequired` give a non-acknowledging provider an
/// explicit terminal outcome. Once terminal, late or duplicate evidence cannot rewrite history.
#[must_use]
pub fn apply_decision_evidence(
    mut state: DecisionSettlementState,
    decision_id: &str,
    evidence: DecisionEvidence,
) -> DecisionEvidenceUpdate {
    let Some(decision) = state.decisions.get_mut(decision_id) else {
        return DecisionEvidenceUpdate {
            state,
            changed: false,
        };
    };
    if decision.phase.is_terminal() {
        return DecisionEvidenceUpdate {
            state,
            changed: false,
        };
    }

    let phase = match evidence {
        DecisionEvidence::ProviderAcknowledged
            if decision.phase == DecisionSettlementPhase::Pending =>
        {
            Some(DecisionSettlementPhase::Acknowledged)
        }
        DecisionEvidence::ProviderSettled => Some(DecisionSettlementPhase::Settled),
        DecisionEvidence::AcknowledgementUnprovable => Some(DecisionSettlementPhase::Unprovable),
        DecisionEvidence::RecoveryRequired => Some(DecisionSettlementPhase::RecoveryRequired),
        DecisionEvidence::ProviderAcknowledged => None,
    };
    let changed = phase.is_some();
    if let Some(phase) = phase {
        decision.phase = phase;
    }
    DecisionEvidenceUpdate { state, changed }
}

#[cfg(test)]
mod tests {
    use gent_types::{DecisionCommand, DecisionSettlementPhase};

    use super::{
        DecisionCommandOutcome, DecisionEvidence, DecisionSettlementState, apply_decision_evidence,
        submit_decision,
    };

    fn command() -> DecisionCommand {
        DecisionCommand {
            decision_id: "decision-1".into(),
            idempotency_key: "key-1".into(),
        }
    }

    #[test]
    fn duplicate_commands_keep_the_original_pending_decision() {
        let accepted = submit_decision(DecisionSettlementState::default(), command());
        let duplicate = submit_decision(accepted.state, command());

        assert!(matches!(
            duplicate.outcome,
            DecisionCommandOutcome::Duplicate(ref decision)
                if decision.phase == DecisionSettlementPhase::Pending
        ));
        assert_eq!(
            duplicate.state.decision("decision-1").unwrap().phase,
            DecisionSettlementPhase::Pending
        );
    }

    #[test]
    fn acknowledgement_then_settlement_is_monotonic_and_idempotent() {
        let accepted = submit_decision(DecisionSettlementState::default(), command());
        let acknowledged = apply_decision_evidence(
            accepted.state,
            "decision-1",
            DecisionEvidence::ProviderAcknowledged,
        );
        let settled = apply_decision_evidence(
            acknowledged.state,
            "decision-1",
            DecisionEvidence::ProviderSettled,
        );
        let late_ack = apply_decision_evidence(
            settled.state,
            "decision-1",
            DecisionEvidence::ProviderAcknowledged,
        );

        assert!(acknowledged.changed);
        assert!(settled.changed);
        assert!(!late_ack.changed);
        assert_eq!(
            late_ack.state.decision("decision-1").unwrap().phase,
            DecisionSettlementPhase::Settled
        );
    }

    #[test]
    fn a_non_acknowledging_provider_reaches_an_explicit_terminal_outcome() {
        let accepted = submit_decision(DecisionSettlementState::default(), command());
        let resolved = apply_decision_evidence(
            accepted.state,
            "decision-1",
            DecisionEvidence::AcknowledgementUnprovable,
        );

        let phase = &resolved.state.decision("decision-1").unwrap().phase;
        assert!(resolved.changed);
        assert_eq!(*phase, DecisionSettlementPhase::Unprovable);
        assert!(phase.is_terminal());
    }

    #[test]
    fn recovery_is_terminal_and_identifier_reuse_is_rejected() {
        let accepted = submit_decision(DecisionSettlementState::default(), command());
        let recovered = apply_decision_evidence(
            accepted.state,
            "decision-1",
            DecisionEvidence::RecoveryRequired,
        );
        let conflict = submit_decision(
            recovered.state,
            DecisionCommand {
                decision_id: "decision-1".into(),
                idempotency_key: "new-key".into(),
            },
        );

        assert!(matches!(
            conflict.outcome,
            DecisionCommandOutcome::DecisionIdConflict { .. }
        ));
        assert_eq!(
            conflict.state.decision("decision-1").unwrap().phase,
            DecisionSettlementPhase::RecoveryRequired
        );
    }
}
