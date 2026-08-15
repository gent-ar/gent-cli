//! Fakeable boundaries for signed runtime-release update orchestration.

use gent_types::{
    RuntimeReleaseArtifact, RuntimeReleaseChannel, RuntimeStagingReceipt, RuntimeUpdateRecord,
    SignedRuntimeRelease,
};

use crate::LedgerError;

/// Error from a release source, staging operation, health probe, or bootstrapper handoff.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUpdatePortError {
    #[error("runtime update operation is unavailable: {0}")]
    Unavailable(String),
    #[error("runtime update integrity check failed: {0}")]
    Integrity(String),
}

/// Source for one signed, channel-selected runtime release manifest.
pub trait RuntimeReleaseSource: Send + Sync {
    /// Reads metadata only; trust verification belongs to the caller's verifier policy.
    ///
    /// # Errors
    /// Returns an error when release metadata cannot be read.
    fn fetch_release(
        &self,
        channel: RuntimeReleaseChannel,
        target: &str,
    ) -> Result<SignedRuntimeRelease, RuntimeUpdatePortError>;
}

/// Stages an already-authorized archive and proves its declared artifact digest.
pub trait RuntimeArtifactStager: Send + Sync {
    ///
    /// # Errors
    /// Returns an error when staging or digest verification fails.
    fn stage(
        &self,
        attempt_id: &str,
        artifact: &RuntimeReleaseArtifact,
    ) -> Result<RuntimeStagingReceipt, RuntimeUpdatePortError>;
}

/// Performs the new binary's bounded handshake and read-only local health probe.
pub trait RuntimeHealthProbe: Send + Sync {
    ///
    /// # Errors
    /// Returns an error when the staged runtime fails a health probe.
    fn probe(&self, staged: &RuntimeStagingReceipt) -> Result<(), RuntimeUpdatePortError>;
}

/// Delegates activation to the platform-specific bootstrapper, never the running daemon itself.
pub trait RuntimeActivation: Send + Sync {
    ///
    /// # Errors
    /// Returns an error when the external bootstrapper cannot accept activation.
    fn activate(&self, staged: &RuntimeStagingReceipt) -> Result<(), RuntimeUpdatePortError>;
}

/// Durable append-only boundary for update-attempt checkpoints.
pub trait RuntimeUpdateJournal: Send + Sync {
    /// Saves a strictly newer update revision while preserving identical retries.
    ///
    /// # Errors
    /// Returns an error when ordering/integrity invariants or storage fail.
    fn save_runtime_update(&self, record: &RuntimeUpdateRecord) -> Result<(), LedgerError>;

    /// Reads the latest update checkpoint for one attempt.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_runtime_update(
        &self,
        attempt_id: &str,
    ) -> Result<Option<RuntimeUpdateRecord>, LedgerError>;
}
