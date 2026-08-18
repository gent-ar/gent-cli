//! Private, uncomposed prompt-triggered provisioning for public provider CLIs.
//!
//! This seam is intentionally unreachable from `gentd` bootstrap and public IPC. A future
//! evidence-approved host supplies a durable accepted receipt and explicit consent after a prompt
//! identifies a missing public provider. Claurst never enters this public npm path.

use std::path::{Path, PathBuf};

use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_protocol::DependencyProvider;
use gent_types::{Receipt, ReceiptStatus};

use crate::node_runtime_lock::{AppNodeRuntimeLock, AppNodeRuntimeLockError};

/// Receipt-bound, consented prompt trigger without any prompt text or provider executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateProvisionRequest {
    pub(crate) receipt: Receipt,
    pub(crate) provider: DependencyProvider,
    pub(crate) consent_granted: bool,
    pub(crate) now_unix_seconds: u64,
}

/// The only results visible to a future private receipt settler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProvisionResult {
    ConsentRequired,
    Installed(ProvisionedProviderLock),
    /// The external effect may have happened, but runtime identity changed before verification.
    /// The future receipt owner must settle this as `Unprovable`, never retry automatically.
    Ambiguous,
}

/// Failure before or during a fixed, signed public-provider package operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateProvisionError {
    #[error("private provisioning requires an accepted durable receipt")]
    ReceiptNotAccepted,
    #[error(transparent)]
    Runtime(#[from] AppNodeRuntimeLockError),
    #[error("signed package policy failed: {0}")]
    Policy(String),
    #[error("signed package policy selected a different provider")]
    ProviderMismatch,
    #[error("fixed npm installer failed: {0}")]
    Installer(String),
    #[error("post-install provider verification is unavailable")]
    VerificationUnavailable,
}

/// Immutable executable/version/digest lock recorded by a future receipt owner before a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProvisionedProviderLock {
    pub(crate) provider: DependencyProvider,
    pub(crate) executable: PathBuf,
    pub(crate) version: String,
    pub(crate) digest_sha256: String,
}

/// Discovers and locks one public provider executable after fixed npm installation.
///
/// This port is the post-effect authority boundary: no provider run may use a package until its
/// implementation has verified the selected executable, exact version, and content digest.
pub(crate) trait ProvisionedProviderVerifier: Clone + Send + Sync {
    /// Returns one immutable lock below the supplied Gent-owned npm prefix.
    ///
    /// # Errors
    /// Returns an error when the executable is missing, changed, or unsupported.
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &Path,
    ) -> Result<ProvisionedProviderLock, String>;
}

/// Private composition edge for an app-supplied Node runtime, policy, and fixed npm installer.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProviderProvisioner<I, P, V> {
    runtime: AppNodeRuntimeLock,
    installer: I,
    policy: P,
    verifier: Option<V>,
}

impl<I, P, V> PrivateProviderProvisioner<I, P, V> {
    #[must_use]
    pub(crate) fn new(
        runtime: AppNodeRuntimeLock,
        installer: I,
        policy: P,
        verifier: Option<V>,
    ) -> Self {
        Self {
            runtime,
            installer,
            policy,
            verifier,
        }
    }
}

impl<I: DependencyInstaller, P: PackageInstallPolicy, V: ProvisionedProviderVerifier>
    PrivateProviderProvisioner<I, P, V>
{
    /// Captures the app runtime without enabling installation or registering a public handler.
    ///
    /// # Errors
    /// Returns an error when `GENT_NODE_BINARY` cannot be locked.
    pub(crate) fn from_environment(
        data_dir: &Path,
        installer: I,
        policy: P,
        verifier: Option<V>,
    ) -> Result<Self, PrivateProvisionError> {
        Ok(Self::new(
            AppNodeRuntimeLock::from_environment(data_dir)?,
            installer,
            policy,
            verifier,
        ))
    }

    /// Runs one consented package installation after exact policy and Node identity checks.
    ///
    /// The Node/npm pair is rechecked immediately before the fixed installer effect. It is checked
    /// again afterwards; any post-effect change returns `Ambiguous` so a receipt owner cannot
    /// claim success or replay the install.
    ///
    /// # Errors
    /// Returns before an effect for invalid receipts, changed pre-effect runtime, or invalid policy;
    /// installer failures are reported without exposing a public command path.
    pub(crate) fn provision(
        &self,
        request: &PrivateProvisionRequest,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        if request.receipt.status != ReceiptStatus::Accepted {
            return Err(PrivateProvisionError::ReceiptNotAccepted);
        }
        if !request.consent_granted {
            return Ok(PrivateProvisionResult::ConsentRequired);
        }
        let verifier = self
            .verifier
            .as_ref()
            .ok_or(PrivateProvisionError::VerificationUnavailable)?;
        let package = self
            .policy
            .approved_package(request.provider.as_str(), request.now_unix_seconds)
            .map_err(|error| PrivateProvisionError::Policy(error.to_string()))?;
        if package.provider != request.provider.as_str() {
            return Err(PrivateProvisionError::ProviderMismatch);
        }
        // Keep the runtime fence adjacent to the only external effect. Policy selection is
        // pure/read-only, so it must not create a gap after this recheck.
        let npm = self.runtime.rechecked_npm_prefix()?;
        self.installer
            .install(&npm, &package)
            .map_err(|error| PrivateProvisionError::Installer(error.to_string()))?;
        if self.runtime.recheck().is_err() {
            return Ok(PrivateProvisionResult::Ambiguous);
        }
        match verifier.lock(request.provider, npm.prefix()) {
            Ok(lock) if valid_lock(&lock, request.provider, npm.prefix()) => {
                Ok(PrivateProvisionResult::Installed(lock))
            }
            Ok(_) | Err(_) => Ok(PrivateProvisionResult::Ambiguous),
        }
    }
}

fn valid_lock(lock: &ProvisionedProviderLock, provider: DependencyProvider, prefix: &Path) -> bool {
    let Ok(executable) = lock.executable.canonicalize() else {
        return false;
    };
    let Ok(prefix) = prefix.canonicalize() else {
        return false;
    };
    lock.provider == provider
        && executable.starts_with(prefix)
        && executable.is_file()
        && !lock.version.trim().is_empty()
        && lock.version.len() <= 512
        && !lock.version.contains('\0')
        && lock.digest_sha256.len() == 64
        && lock.digest_sha256.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
}

#[cfg(test)]
#[path = "private_provider_provisioning_tests.rs"]
mod tests;
