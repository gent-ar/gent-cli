//! Coordinator orchestration for the durable conversation → run → turn hierarchy.

use gent_core::permits_turn_transition;
use gent_ports::{ConversationLedger, Ledger, LedgerError, RunRecord, TurnPhaseUpdate};
use gent_types::{ConversationRecord, DurableTurnPhase, TurnRecord};

use crate::{Coordinator, RuntimeError, to_record};

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger,
{
    /// Atomically creates a conversation and its immutable root run.
    ///
    /// # Errors
    /// Returns an error when the root does not name the conversation or persistence fails.
    pub fn create_conversation_run(
        &self,
        conversation: &ConversationRecord,
        run: &gent_core::Run,
    ) -> Result<(), RuntimeError> {
        self.ledger
            .create_conversation_run(conversation, &to_record(run))?;
        Ok(())
    }

    /// Creates an immutable turn within an existing conversation run.
    ///
    /// # Errors
    /// Returns an error when the conversation/run relationship or sequence is invalid.
    pub fn create_turn(&self, turn: &TurnRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_turn(turn)?)
    }

    /// Moves a turn through its pure monotonic lifecycle policy.
    ///
    /// # Errors
    /// Returns an error when the turn is unknown, stale, or the transition is invalid.
    pub fn transition_turn(
        &self,
        turn_id: &str,
        expected: DurableTurnPhase,
        next: DurableTurnPhase,
    ) -> Result<TurnPhaseUpdate, RuntimeError> {
        if !permits_turn_transition(expected, next) {
            return Err(RuntimeError::Ledger(LedgerError::Invariant(
                "durable turn transition is not permitted".into(),
            )));
        }
        Ok(self.ledger.replace_turn_phase(turn_id, expected, next)?)
    }

    /// Lists immutable run lineage for a conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn conversation_runs(&self, conversation_id: &str) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(self.ledger.list_conversation_runs(conversation_id)?)
    }

    /// Lists durable turns in sequence order for a run.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn run_turns(&self, run_id: &str) -> Result<Vec<TurnRecord>, RuntimeError> {
        Ok(self.ledger.list_run_turns(run_id)?)
    }
}
