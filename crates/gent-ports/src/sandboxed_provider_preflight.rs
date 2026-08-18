//! Private daemon boundary for sandbox preparation before a provider spawn.
//!
//! This is deliberately a preflight port: it neither starts a process nor exposes a transport
//! capability. A future composition must use its attestation as a prerequisite to launch.

use gent_types::{SandboxLaunchAttestation, SandboxedLaunchRequest};

/// Controlled sandbox-preflight failures, with no provider-native output or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SandboxedProviderPreflightError {
    #[error("a native sandbox backend is unavailable")]
    Unavailable,
    #[error("sandbox profile was rejected")]
    ProfileRejected,
    #[error("provider executable changed before sandbox preparation")]
    LockChanged,
    #[error("sandbox backend failed containment attestation")]
    AttestationRejected,
}

/// Daemon-owned sandbox preparation boundary for public provider execution.
///
/// An implementation must rediscover and exactly recheck the executable lock immediately before
/// preparing the native sandbox. It returns an attestation only when containment is enforced. It
/// must not spawn the provider; process launch remains a separate future edge that consumes this
/// attestation atomically.
pub trait SandboxedProviderPreflight: Send + Sync {
    /// Prepares containment and binds it to the immutable executable and profile.
    ///
    /// # Errors
    /// Returns a controlled error with zero provider spawn when preflight cannot prove every gate.
    fn preflight(
        &self,
        request: &SandboxedLaunchRequest,
    ) -> Result<SandboxLaunchAttestation, SandboxedProviderPreflightError>;
}
