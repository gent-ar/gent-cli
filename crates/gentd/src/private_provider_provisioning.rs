//! Private, uncomposed prompt-triggered provisioning for public provider CLIs.
//!
//! This seam is intentionally unreachable from `gentd` bootstrap and public IPC. A future
//! evidence-approved host supplies a durable accepted receipt and explicit consent after a prompt
//! identifies a missing public provider. Claurst never enters this public npm path.

use std::{collections::BTreeMap, path::Path, path::PathBuf};

use ed25519_dalek::VerifyingKey;
use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};
pub(crate) use gent_types::ProvisionedProviderLock;
#[cfg(test)]
use gent_types::ReceiptStatus;
use gent_types::{ProviderPromptProvisionCommandBinding, ProvisionedProviderInstallation, Receipt};

pub(crate) use crate::private_provider_provisioning_error::PrivateProvisionError;
use crate::{
    node_runtime_lock::AppNodeRuntimeLock,
    private_provider_compatibility::ProvisionedProviderCompatibility,
};

/// Where and how to re-verify the signed ordinary-authority release immediately before an npm
/// effect. Absent by default; only a future explicit composition supplies this.
#[derive(Clone, Debug)]
pub(crate) struct ReleaseAuthorityConfig {
    pub(crate) path: PathBuf,
    pub(crate) root_keys: BTreeMap<String, VerifyingKey>,
}

mod effect;

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
    /// A pre-effect authority check rejected the request; no provider operation was run.
    Rejected,
    Installed(Box<ProvisionedProviderInstallation>),
    /// The external effect may have happened, but runtime identity changed before verification.
    /// The future receipt owner must settle this as `Unprovable`, never retry automatically.
    Ambiguous,
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
pub(crate) struct PrivateProviderProvisioner<I, P, V, R, B> {
    runtime: AppNodeRuntimeLock,
    installer: I,
    policy: P,
    verifier: Option<V>,
    receipts: R,
    compatibility: B,
    release_authority: Option<ReleaseAuthorityConfig>,
}

impl<I, P, V, R, B> PrivateProviderProvisioner<I, P, V, R, B> {
    #[must_use]
    pub(crate) fn with_compatibility(
        runtime: AppNodeRuntimeLock,
        installer: I,
        policy: P,
        verifier: Option<V>,
        receipts: R,
        compatibility: B,
        release_authority: Option<ReleaseAuthorityConfig>,
    ) -> Self {
        Self {
            runtime,
            installer,
            policy,
            verifier,
            receipts,
            compatibility,
            release_authority,
        }
    }
}

#[cfg(test)]
impl<I, P, V, R>
    PrivateProviderProvisioner<
        I,
        P,
        V,
        R,
        crate::private_provider_compatibility::TestProvisionedProviderCompatibility,
    >
{
    pub(crate) fn new(
        runtime: AppNodeRuntimeLock,
        installer: I,
        policy: P,
        verifier: Option<V>,
        receipts: R,
    ) -> Self {
        Self::with_compatibility(
            runtime,
            installer,
            policy,
            verifier,
            receipts,
            crate::private_provider_compatibility::TestProvisionedProviderCompatibility,
            None,
        )
    }

    pub(crate) fn with_release_authority(
        runtime: AppNodeRuntimeLock,
        installer: I,
        policy: P,
        verifier: Option<V>,
        receipts: R,
        release_authority: ReleaseAuthorityConfig,
    ) -> Self {
        Self::with_compatibility(
            runtime,
            installer,
            policy,
            verifier,
            receipts,
            crate::private_provider_compatibility::TestProvisionedProviderCompatibility,
            Some(release_authority),
        )
    }
}

impl<
    I: DependencyInstaller,
    P: PackageInstallPolicy,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
> PrivateProviderProvisioner<I, P, V, R, B>
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
        compatibility: B,
        release_authority: Option<ReleaseAuthorityConfig>,
    ) -> Result<Self, PrivateProvisionError> {
        Ok(Self::with_compatibility(
            AppNodeRuntimeLock::from_environment(data_dir)?,
            installer,
            policy,
            verifier,
            receipts,
            compatibility,
            release_authority,
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
        let command = gent_runtime::dependency_action_command(&DependencyActionRequest {
            receipt_id: request.receipt.receipt_id.clone(),
            idempotency_key: request.receipt.idempotency_key.clone(),
            host_epoch: request.receipt.host_epoch,
            provider: request.provider,
            action: request.action,
            consent_granted: request.consent_granted,
            reviewed_plan_digest: request.reviewed_plan_digest.clone(),
        });
        self.provision_with_command(request, &command)
    }

    /// Runs an installation only after re-reading this exact daemon-built command.
    ///
    /// # Errors
    /// Returns before an effect when the receipt or command identity is no longer durable.
    pub(crate) fn provision_with_command(
        &self,
        request: &PrivateProvisionRequest,
        command: &gent_types::Command,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        effect::provision(self, request, command)
    }

    /// Runs a prompt-scoped installation only when its complete private command binding matches.
    ///
    /// # Errors
    /// Returns before an effect if the binding, durable command, or freshly selected package
    /// differs from the exact daemon-derived review.
    pub(crate) fn provision_prompt_with_command(
        &self,
        request: &PrivateProvisionRequest,
        command: &gent_types::Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        effect::provision_prompt(self, request, command, binding)
    }
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

#[cfg(test)]
#[path = "private_provider_provisioning_release_tests.rs"]
mod release_tests;

#[cfg(test)]
#[path = "private_provider_provisioning_compatibility_tests.rs"]
mod compatibility_tests;
