//! Daemon-owned discovery boundary for a public provider executable lock.

use gent_types::RunVersionLock;

use crate::PublicProviderRunError;

/// Resolves one allowed public provider to a fully observed immutable lock.
///
/// The resolver owns executable discovery, identity capture, version observation, and compatibility
/// entry selection. Runtime callers must never derive a lock from IPC-supplied path or version data.
pub trait PublicProviderResolver: Send + Sync {
    /// Returns a lock created entirely from daemon-owned observations.
    ///
    /// # Errors
    /// Returns a controlled denial when no currently authorized installation can be resolved.
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError>;
}
