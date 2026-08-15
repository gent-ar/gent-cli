//! Durable identity and lifecycle boundary for conversations and turns.

use gent_types::{ConversationRecord, DurableTurnPhase, TurnRecord};

use crate::{LedgerError, RunRecord};

/// Result of an optimistic durable-turn lifecycle update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnPhaseUpdate {
    Applied(TurnRecord),
    Current(TurnRecord),
}

/// Persistence boundary for the immutable conversation → run → turn hierarchy.
pub trait ConversationLedger: Send + Sync {
    /// Atomically creates a conversation and its root run.
    ///
    /// # Errors
    /// Returns an error when identities do not match, the run is not a root, or persistence fails.
    fn create_conversation_run(
        &self,
        conversation: &ConversationRecord,
        run: &RunRecord,
    ) -> Result<(), LedgerError>;

    /// Reads a durable conversation identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationRecord>, LedgerError>;

    /// Creates an immutable turn assigned to an existing conversation run.
    ///
    /// # Errors
    /// Returns an error when hierarchy, sequence, or persistence invariants fail.
    fn create_turn(&self, turn: &TurnRecord) -> Result<(), LedgerError>;

    /// Reads one durable turn.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_turn(&self, turn_id: &str) -> Result<Option<TurnRecord>, LedgerError>;

    /// Lists the immutable runs belonging to one conversation in creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_conversation_runs(&self, conversation_id: &str) -> Result<Vec<RunRecord>, LedgerError>;

    /// Lists turns belonging to one run in their durable sequence order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_run_turns(&self, run_id: &str) -> Result<Vec<TurnRecord>, LedgerError>;

    /// Advances a turn only when its stored phase still equals `expected`.
    ///
    /// # Errors
    /// Returns an error when the turn is unknown or durable persistence fails.
    fn replace_turn_phase(
        &self,
        turn_id: &str,
        expected: DurableTurnPhase,
        next: DurableTurnPhase,
    ) -> Result<TurnPhaseUpdate, LedgerError>;
}
