//! Exact-command private npm effect, split from the composition seam for source-size limits.

use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_types::{
    Command, ProviderInstallProvenance, ProvisionedProviderInstallation, ReceiptStatus,
};

use super::{
    PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
    PrivateProvisionResult, ProvisionReceiptReader, ProvisionedProviderLock,
    ProvisionedProviderVerifier,
};
use crate::{
    private_provider_compatibility::ProvisionedProviderCompatibility,
    private_provider_lock_validation::{valid_digest, valid_lock},
};

pub(super) fn provision<I, P, V, R, B>(
    provisioner: &PrivateProviderProvisioner<I, P, V, R, B>,
    request: &PrivateProvisionRequest,
    command: &Command,
) -> Result<PrivateProvisionResult, PrivateProvisionError>
where
    I: DependencyInstaller,
    P: PackageInstallPolicy,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
{
    if request.receipt.status != ReceiptStatus::Accepted {
        return Err(PrivateProvisionError::ReceiptNotAccepted);
    }
    if !request.consent_granted {
        return Ok(PrivateProvisionResult::ConsentRequired);
    }
    let same_receipt = command.receipt_id == request.receipt.receipt_id
        && command.idempotency_key == request.receipt.idempotency_key
        && command.host_epoch == request.receipt.host_epoch;
    if !same_receipt {
        return Err(PrivateProvisionError::ReceiptMismatch);
    }
    let verifier = provisioner
        .verifier
        .as_ref()
        .ok_or(PrivateProvisionError::VerificationUnavailable)?;
    let package = provisioner
        .policy
        .approved_package(request.provider.as_str(), request.now_unix_seconds)
        .map_err(|error| PrivateProvisionError::Policy(error.to_string()))?;
    if package.provider != request.provider.as_str() {
        return Err(PrivateProvisionError::ProviderMismatch);
    }
    if !valid_digest(&package.package_policy_digest_sha256) {
        return Err(PrivateProvisionError::PolicyIdentity);
    }
    let npm = provisioner.runtime.rechecked_npm_prefix()?;
    let durable = provisioner
        .receipts
        .accepted_receipt(command)
        .map_err(PrivateProvisionError::ReceiptUnavailable)?;
    if durable != request.receipt || durable.status != ReceiptStatus::Accepted {
        return Err(PrivateProvisionError::ReceiptMismatch);
    }
    provisioner
        .installer
        .install(&npm, &package)
        .map_err(|error| PrivateProvisionError::Installer(error.to_string()))?;
    if provisioner.runtime.recheck().is_err() {
        return Ok(PrivateProvisionResult::Ambiguous);
    }
    match verifier
        .lock(request.provider, npm.prefix())
        .and_then(|lock| {
            valid_lock(&lock, request.provider, npm.prefix())
                .then_some(lock)
                .ok_or_else(|| "private provider executable is invalid".into())
        })
        .and_then(|lock| {
            provisioner
                .compatibility
                .bind(lock.run_lock, request.now_unix_seconds)
        }) {
        Ok(run_lock) => Ok(PrivateProvisionResult::Installed(Box::new(
            ProvisionedProviderInstallation {
                lock: ProvisionedProviderLock { run_lock },
                provenance: ProviderInstallProvenance {
                    package_name: package.package_name,
                    package_version: package.version,
                    package_integrity: package.integrity,
                    package_policy_digest_sha256: package.package_policy_digest_sha256,
                    node_runtime_digest_sha256: provisioner.runtime.node_digest_sha256().into(),
                },
            },
        ))),
        Err(_) => Ok(PrivateProvisionResult::Ambiguous),
    }
}
