//! Durable boundary for provider-neutral user-authored conversation goals.

use gent_types::{GoalBinding, GoalRecord};

use crate::LedgerError;

/// The result of one atomic goal create or compare-and-replace attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalWrite {
    Created(GoalRecord),
    Updated(GoalRecord),
    Current(GoalRecord),
}

/// Persistence boundary for revision-fenced goal records.
pub trait GoalLedger: Send + Sync {
    /// Reads one durable goal by its complete ownership binding.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_goal(&self, binding: &GoalBinding) -> Result<Option<GoalRecord>, LedgerError>;

    /// Atomically creates a revision-one goal or returns the existing record.
    ///
    /// # Errors
    /// Returns an error when persistence fails or the record violates durable invariants.
    fn create_goal(&self, goal: &GoalRecord) -> Result<GoalWrite, LedgerError>;

    /// Atomically replaces an exact current revision or returns the durable current record.
    ///
    /// # Errors
    /// Returns an error when persistence fails or the replacement changes immutable identity.
    fn replace_goal(
        &self,
        expected: &GoalRecord,
        next: &GoalRecord,
    ) -> Result<GoalWrite, LedgerError>;

    /// Reads the bounded current goals in one conversation, ordered by stable creation identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn conversation_goals(&self, conversation_id: &str) -> Result<Vec<GoalRecord>, LedgerError>;
}
