//! Public-provider process lifecycle port and its controlled error surface.

use gent_types::RunVersionLock;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum PublicProviderRunError {
    #[error("public provider executable is not authorized by signed compatibility evidence")]
    CompatibilityDenied,
    #[error("provider executable changed before spawn or resume")]
    ProviderChanged,
    #[error("provider run is not active")]
    NotActive,
    #[error("provider lifecycle failed: {0}")]
    Failed(String),
}

/// Daemon-owned public provider lifecycle boundary.
///
/// Implementations receive only Claude or Codex locks. Private bridges are represented separately
/// by `ExternalProviderBridge` and cannot enter this lifecycle.
pub trait PublicProviderRunner: Send + Sync {
    /// Starts a new process only after the caller has durably reserved its run.
    ///
    /// # Errors
    /// Returns a changed executable or controlled launcher failure without substitution.
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError>;
    /// Resumes a process only after the caller has re-established durable ownership.
    ///
    /// # Errors
    /// Returns a changed executable or controlled launcher failure without substitution.
    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError>;
    /// Interrupts the complete process tree currently owned by `run_id`.
    ///
    /// # Errors
    /// Returns an inactive-run or process-tree interruption failure.
    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
}
