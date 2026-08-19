//! Failure classification for the private, uncomposed prompt-triggered provisioning seam.

use crate::node_runtime_lock::AppNodeRuntimeLockError;

/// Failure before or during a fixed, signed public-provider package operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateProvisionError {
    #[error("private provisioning requires an accepted durable receipt")]
    ReceiptNotAccepted,
    #[error("durable provisioning receipt is unavailable: {0}")]
    ReceiptUnavailable(String),
    #[error("durable provisioning receipt no longer matches the accepted request")]
    ReceiptMismatch,
    #[error("prompt provision command does not match its daemon-derived binding")]
    PromptBindingMismatch,
    #[error("signed package policy changed after the reviewed prompt binding")]
    PromptPackageMismatch,
    #[error(transparent)]
    Runtime(#[from] AppNodeRuntimeLockError),
    #[error("signed package policy failed: {0}")]
    Policy(String),
    #[error("signed package policy selected a different provider")]
    ProviderMismatch,
    #[error("signed package policy identity is invalid")]
    PolicyIdentity,
    #[error("fixed npm installer failed: {0}")]
    Installer(String),
    #[error("post-install provider verification is unavailable")]
    VerificationUnavailable,
    #[error(
        "prompt-scoped provisioning requires an explicitly composed release reauthorization source"
    )]
    ReleaseAuthorityUnavailable,
    #[error(
        "signed ordinary authority release failed reauthorization immediately before this effect: {0}"
    )]
    ReleaseReauthorizationFailed(String),
    #[error("signed ordinary authority release no longer matches the digest bound at admission")]
    ReleaseDigestMismatch,
}

impl PrivateProvisionError {
    /// Whether the effect boundary was certainly not reached.
    ///
    /// An installer failure is deliberately excluded: npm may have changed the private prefix
    /// before returning an error, so its receipt must remain unprovable rather than retryable.
    /// A release-authority mismatch or reauthorization failure is likewise excluded: both are a
    /// potential tamper/revocation signal, not an ordinary pre-effect business-rule mismatch, so
    /// they must settle the same manual, non-automatic way as a genuinely ambiguous post-effect
    /// state, never be silently retried.
    pub(crate) const fn is_definitely_pre_effect(&self) -> bool {
        match self {
            Self::ReceiptNotAccepted
            | Self::ReceiptUnavailable(_)
            | Self::ReceiptMismatch
            | Self::PromptBindingMismatch
            | Self::PromptPackageMismatch
            | Self::Runtime(_)
            | Self::Policy(_)
            | Self::ProviderMismatch
            | Self::PolicyIdentity
            | Self::VerificationUnavailable
            | Self::ReleaseAuthorityUnavailable => true,
            Self::Installer(_)
            | Self::ReleaseReauthorizationFailed(_)
            | Self::ReleaseDigestMismatch => false,
        }
    }
}
