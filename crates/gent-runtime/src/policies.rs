//! Coordinator calls for durable, versioned provider permission policy.

use gent_ports::PolicyLedger;
use gent_types::{PolicyRecord, PolicyScope};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + PolicyLedger,
{
    /// Persists the next immutable workspace policy revision.
    ///
    /// # Errors
    /// Returns an error when the policy is invalid, out of sequence, or cannot persist.
    pub fn save_policy(&self, policy: &PolicyRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.save_policy(policy)?)
    }

    /// Reads the latest policy without exposing any provider credentials or endpoint data.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn current_policy(
        &self,
        workspace_id: &str,
        scope: PolicyScope,
    ) -> Result<Option<PolicyRecord>, RuntimeError> {
        Ok(self.ledger.current_policy(workspace_id, scope)?)
    }
}
