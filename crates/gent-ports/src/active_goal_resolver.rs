//! Read-only daemon boundary for deriving one active user goal per durable run.

use gent_types::GoalProjection;

use crate::LedgerError;

/// Resolves the current provider-neutral goal context from Gent-owned durable state.
///
/// Implementations must never accept a provider-supplied goal. `None` means that no valid active
/// goal is bound to the exact conversation and run, or that composition deliberately withholds it.
pub trait ActiveGoalResolver: Send + Sync + std::fmt::Debug {
    /// Resolves one fresh immutable projection immediately before a daemon-owned provider turn.
    ///
    /// # Errors
    /// Returns an invariant or storage error when matching active goal state is unsafe to use.
    fn resolve_active_goal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<GoalProjection>, LedgerError>;
}
