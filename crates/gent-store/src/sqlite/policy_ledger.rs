//! Adapter joining immutable policy queries to the public persistence port.

use gent_ports::{LedgerError, PolicyLedger};
use gent_types::{PolicyRecord, PolicyScope};

use super::{SqliteLedger, policies};

impl PolicyLedger for SqliteLedger {
    fn ensure_default_provider_permission_policy(
        &self,
        workspace_id: &str,
    ) -> Result<PolicyRecord, LedgerError> {
        policies::ensure_default_provider_permission_policy(self, workspace_id)
    }

    fn save_policy(&self, policy: &PolicyRecord) -> Result<(), LedgerError> {
        policies::save(self, policy)
    }

    fn current_policy(
        &self,
        workspace_id: &str,
        scope: PolicyScope,
    ) -> Result<Option<PolicyRecord>, LedgerError> {
        policies::current_policy(self, workspace_id, scope)
    }
}
