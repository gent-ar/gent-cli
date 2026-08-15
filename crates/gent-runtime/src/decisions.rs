//! Durable orchestration for the pure decision-settlement reducer.

use gent_core::{
    DecisionCommandOutcome, DecisionEvidence, DecisionSettlementState, apply_decision_evidence,
    submit_decision,
};
use gent_ports::{DecisionClaim, DecisionPhaseUpdate, Ledger};
use gent_types::{DecisionCommand, DecisionSettlement};

use crate::{Coordinator, RuntimeError};

impl<L: Ledger> Coordinator<L> {
    /// Persists a client decision exactly once, preserving the reducer's conflict outcomes.
    ///
    /// # Errors
    /// Returns an error when durable decision state cannot be read or written.
    pub fn submit_decision(
        &self,
        command: DecisionCommand,
    ) -> Result<DecisionCommandOutcome, RuntimeError> {
        let proposed = submit_decision(DecisionSettlementState::default(), command.clone());
        let outcome = match self.ledger.claim_decision(&command)? {
            DecisionClaim::Created(decision) => match proposed.outcome {
                DecisionCommandOutcome::Accepted(_) => DecisionCommandOutcome::Accepted(decision),
                _ => unreachable!("an empty decision state always accepts a new command"),
            },
            DecisionClaim::Existing(decision) => {
                submit_decision(DecisionSettlementState::from_decision(decision), command).outcome
            }
        };
        Ok(outcome)
    }

    /// Records provider or crash-recovery evidence without allowing terminal history to change.
    ///
    /// # Errors
    /// Returns an error when no matching decision exists, persistence fails, or contention persists.
    pub fn apply_decision_evidence(
        &self,
        decision_id: &str,
        evidence: DecisionEvidence,
    ) -> Result<DecisionSettlement, RuntimeError> {
        for _ in 0..3 {
            let current = self
                .ledger
                .find_decision(decision_id)?
                .ok_or_else(|| RuntimeError::UnknownDecision(decision_id.into()))?;
            let update = apply_decision_evidence(
                DecisionSettlementState::from_decision(current.clone()),
                decision_id,
                evidence,
            );
            if !update.changed {
                return Ok(current);
            }
            let next = update.state.decision(decision_id).ok_or_else(|| {
                RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                    "decision reducer removed known decision".into(),
                ))
            })?;
            match self
                .ledger
                .replace_decision_phase(decision_id, &current.phase, &next.phase)?
            {
                DecisionPhaseUpdate::Applied(decision) => return Ok(decision),
                DecisionPhaseUpdate::Current(_) => {}
            }
        }
        Err(RuntimeError::DecisionContention)
    }

    /// Reads a durable decision without exposing a storage implementation.
    ///
    /// # Errors
    /// Returns an error when durable decision state cannot be read.
    pub fn decision(&self, decision_id: &str) -> Result<Option<DecisionSettlement>, RuntimeError> {
        Ok(self.ledger.find_decision(decision_id)?)
    }
}

#[cfg(test)]
mod tests {
    use gent_core::{DecisionCommandOutcome, DecisionEvidence};
    use gent_store::SqliteLedger;
    use gent_types::{CapabilitySet, DecisionCommand, DecisionSettlementPhase};

    use crate::Coordinator;

    fn command(id: &str, key: &str) -> DecisionCommand {
        DecisionCommand {
            decision_id: id.into(),
            idempotency_key: key.into(),
        }
    }

    #[test]
    fn durable_decisions_deduplicate_then_settle_monotonically() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        assert!(matches!(
            coordinator.submit_decision(command("d1", "key-1")).unwrap(),
            DecisionCommandOutcome::Accepted(_)
        ));
        assert!(matches!(
            coordinator.submit_decision(command("d1", "key-1")).unwrap(),
            DecisionCommandOutcome::Duplicate(_)
        ));
        assert_eq!(
            coordinator
                .apply_decision_evidence("d1", DecisionEvidence::ProviderAcknowledged)
                .unwrap()
                .phase,
            DecisionSettlementPhase::Acknowledged
        );
        assert_eq!(
            coordinator
                .apply_decision_evidence("d1", DecisionEvidence::ProviderSettled)
                .unwrap()
                .phase,
            DecisionSettlementPhase::Settled
        );
        assert_eq!(
            coordinator
                .apply_decision_evidence("d1", DecisionEvidence::RecoveryRequired)
                .unwrap()
                .phase,
            DecisionSettlementPhase::Settled
        );
    }

    #[test]
    fn durable_decision_conflicts_preserve_idempotency_key_precedence() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        coordinator
            .submit_decision(command("one", "key-1"))
            .unwrap();
        coordinator
            .submit_decision(command("two", "key-2"))
            .unwrap();
        assert!(matches!(
            coordinator.submit_decision(command("one", "key-2")).unwrap(),
            DecisionCommandOutcome::IdempotencyConflict { existing_decision_id }
                if existing_decision_id == "two"
        ));
        assert!(matches!(
            coordinator.submit_decision(command("one", "key-3")).unwrap(),
            DecisionCommandOutcome::DecisionIdConflict { existing_idempotency_key }
                if existing_idempotency_key == "key-1"
        ));
    }

    #[test]
    fn terminal_unprovable_and_recovery_survive_sqlite_restart() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();
        {
            let coordinator =
                Coordinator::new(SqliteLedger::open(&path).unwrap(), CapabilitySet::default());
            coordinator
                .submit_decision(command("unprovable", "key-1"))
                .unwrap();
            coordinator
                .submit_decision(command("recovery", "key-2"))
                .unwrap();
            coordinator
                .apply_decision_evidence("unprovable", DecisionEvidence::AcknowledgementUnprovable)
                .unwrap();
            coordinator
                .apply_decision_evidence("recovery", DecisionEvidence::RecoveryRequired)
                .unwrap();
        }
        let reopened =
            Coordinator::new(SqliteLedger::open(path).unwrap(), CapabilitySet::default());
        assert_eq!(
            reopened.decision("unprovable").unwrap().unwrap().phase,
            DecisionSettlementPhase::Unprovable
        );
        assert_eq!(
            reopened.decision("recovery").unwrap().unwrap().phase,
            DecisionSettlementPhase::RecoveryRequired
        );
    }
}
