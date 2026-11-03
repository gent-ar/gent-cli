//! Durable boundary for immutable workspace permission-policy revisions.

use gent_types::{PolicyRecord, PolicyScope};

use crate::LedgerError;

/// Persistence boundary for versioned, secret-free policy records.
pub trait PolicyLedger: Send + Sync {
    fn ensure_default_provider_permission_policy(
        &self,
        workspace_id: &str,
    ) -> Result<PolicyRecord, LedgerError> {
        let _ = workspace_id;
        Err(LedgerError::Invariant(
            "default provider permission policy is unavailable".into(),
        ))
    }

    /// Saves an immutable policy revision under an existing workspace.
    ///
    /// # Errors
    /// Returns an error when the record is invalid, not next in sequence, or cannot persist.
    fn save_policy(&self, policy: &PolicyRecord) -> Result<(), LedgerError>;

    /// Atomically saves a user-facing provider policy only if containment-bound
    /// active work cannot be invalidated. Implementations must serialize this
    /// check with durable prompt admission.
    /// # Errors
    /// Returns an error if active work prevents a transition or persistence fails.
    fn save_provider_permission_policy(&self, policy: &PolicyRecord) -> Result<(), LedgerError> {
        let current = self.current_policy(&policy.workspace_id, policy.scope)?;
        if policy.mode.is_unattended()
            || current
                .as_ref()
                .is_some_and(|record| record.mode.is_unattended())
        {
            return Err(LedgerError::Invariant(
                "atomic contained permission transition is unavailable".into(),
            ));
        }
        self.save_policy(policy)
    }

    /// Reads the latest policy revision for one workspace and scope.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn current_policy(
        &self,
        workspace_id: &str,
        scope: PolicyScope,
    ) -> Result<Option<PolicyRecord>, LedgerError>;
}
