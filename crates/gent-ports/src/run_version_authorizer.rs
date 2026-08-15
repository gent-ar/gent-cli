//! Fail-closed authorization for immutable public-provider executable locks.

use gent_types::RunVersionLock;

use crate::PublicProviderRunError;

/// Confirms a captured executable lock is authorized for a public provider lifecycle action.
///
/// Implementations own trust material, such as a verified signed compatibility cache. Runtime
/// policy calls this boundary before durable reservation and immediately before process resume.
pub trait RunVersionAuthorizer: Send + Sync {
    /// Rejects untrusted, expired, revoked, or otherwise unauthorized executable locks.
    ///
    /// # Errors
    /// Returns `CompatibilityDenied` when the lock lacks currently trusted evidence.
    fn authorize(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError>;
}
