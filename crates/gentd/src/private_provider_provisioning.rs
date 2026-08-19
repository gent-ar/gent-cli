//! Private, uncomposed prompt-triggered provisioning for public provider CLIs.
//!
//! This seam is intentionally unreachable from `gentd` bootstrap and public IPC. A future
//! evidence-approved host supplies a durable accepted receipt and explicit consent after a prompt
//! identifies a missing public provider. Claurst never enters this public npm path.

use std::path::Path;

use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};
pub(crate) use gent_types::ProvisionedProviderLock;
use gent_types::{
    ProviderInstallProvenance, ProvisionedProviderInstallation, Receipt, ReceiptStatus,
};

use crate::node_runtime_lock::{AppNodeRuntimeLock, AppNodeRuntimeLockError};

/// Receipt-bound, consented prompt trigger without any prompt text or provider executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateProvisionRequest {
    pub(crate) receipt: Receipt,
    pub(crate) provider: DependencyProvider,
    pub(crate) action: DependencyAction,
    pub(crate) reviewed_plan_digest: String,
    pub(crate) consent_granted: bool,
    pub(crate) now_unix_seconds: u64,
}

/// The only results visible to a future private receipt settler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProvisionResult {
    ConsentRequired,
    Installed(Box<ProvisionedProviderInstallation>),
    /// The external effect may have happened, but runtime identity changed before verification.
    /// The future receipt owner must settle this as `Unprovable`, never retry automatically.
    Ambiguous,
}

/// Failure before or during a fixed, signed public-provider package operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateProvisionError {
    #[error("private provisioning requires an accepted durable receipt")]
    ReceiptNotAccepted,
    #[error("durable provisioning receipt is unavailable: {0}")]
    ReceiptUnavailable(String),
    #[error("durable provisioning receipt no longer matches the accepted request")]
    ReceiptMismatch,
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

/// Re-reads the one durable accepted dependency-action command allowed to cause an npm effect.
///
/// This port keeps receipt ownership in the future ledger composition.  A caller-provided
/// The receipt is only a claim: the complete command must match current durable state before npm.
pub(crate) trait ProvisionReceiptReader: Clone + Send + Sync {
    /// Returns the current accepted receipt for this complete dependency-action command.
    fn accepted_receipt(&self, command: &gent_types::Command) -> Result<Receipt, String>;
}

/// Private composition edge for an app-supplied Node runtime, policy, and fixed npm installer.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProviderProvisioner<I, P, V, R> {
    runtime: AppNodeRuntimeLock,
    installer: I,
    policy: P,
    verifier: Option<V>,
    receipts: R,
}

impl<I, P, V, R> PrivateProviderProvisioner<I, P, V, R> {
    #[must_use]
    pub(crate) fn new(
        runtime: AppNodeRuntimeLock,
        installer: I,
        policy: P,
        verifier: Option<V>,
        receipts: R,
    ) -> Self {
        Self {
            runtime,
            installer,
            policy,
            verifier,
            receipts,
        }
    }
}

impl<
    I: DependencyInstaller,
    P: PackageInstallPolicy,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
> PrivateProviderProvisioner<I, P, V, R>
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
        receipts: R,
    ) -> Result<Self, PrivateProvisionError> {
        Ok(Self::new(
            AppNodeRuntimeLock::from_environment(data_dir)?,
            installer,
            policy,
            verifier,
            receipts,
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
        let command = gent_runtime::dependency_action_command(&DependencyActionRequest {
            receipt_id: request.receipt.receipt_id.clone(),
            idempotency_key: request.receipt.idempotency_key.clone(),
            host_epoch: request.receipt.host_epoch,
            provider: request.provider,
            action: request.action,
            consent_granted: request.consent_granted,
            reviewed_plan_digest: request.reviewed_plan_digest.clone(),
        });
        let durable_receipt = self
            .receipts
            .accepted_receipt(&command)
            .map_err(PrivateProvisionError::ReceiptUnavailable)?;
        if durable_receipt != request.receipt || durable_receipt.status != ReceiptStatus::Accepted {
            return Err(PrivateProvisionError::ReceiptMismatch);
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
        if !valid_digest(&package.package_policy_digest_sha256) {
            return Err(PrivateProvisionError::PolicyIdentity);
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
            Ok(lock) if valid_lock(&lock, request.provider, npm.prefix()) => Ok(
                PrivateProvisionResult::Installed(Box::new(ProvisionedProviderInstallation {
                    lock,
                    provenance: ProviderInstallProvenance {
                        package_name: package.package_name,
                        package_version: package.version,
                        package_integrity: package.integrity,
                        package_policy_digest_sha256: package.package_policy_digest_sha256,
                        node_runtime_digest_sha256: self.runtime.node_digest_sha256().into(),
                    },
                })),
            ),
            Ok(_) | Err(_) => Ok(PrivateProvisionResult::Ambiguous),
        }
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_lock(lock: &ProvisionedProviderLock, provider: DependencyProvider, prefix: &Path) -> bool {
    let Ok(executable) = Path::new(&lock.run_lock.canonical_path).canonicalize() else {
        return false;
    };
    let Ok(prefix) = prefix.canonicalize() else {
        return false;
    };
    lock.run_lock.provider == provider.as_str()
        && executable.starts_with(prefix)
        && executable.is_file()
        && executable.display().to_string() == lock.run_lock.canonical_path
        && valid_version(&lock.run_lock.version)
        && lock.run_lock.digest_sha256.len() == 64
        && lock.run_lock.digest_sha256.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
        && gent_drivers::lock::recheck(&lock.run_lock).is_ok()
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 512
        && version.trim() == version
        && !version.contains('\0')
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct TestAcceptedReceiptReader;

#[cfg(test)]
impl ProvisionReceiptReader for TestAcceptedReceiptReader {
    fn accepted_receipt(&self, command: &gent_types::Command) -> Result<Receipt, String> {
        Ok(Receipt {
            receipt_id: command.receipt_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            status: ReceiptStatus::Accepted,
            host_epoch: command.host_epoch,
        })
    }
}

#[cfg(test)]
#[path = "private_provider_provisioning_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "private_provider_provisioning_error_tests.rs"]
mod error_tests;

#[cfg(test)]
#[path = "private_provider_provisioning_receipt_tests.rs"]
mod receipt_tests;
