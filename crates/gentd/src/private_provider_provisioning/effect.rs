//! Exact-command private npm effect, split from the composition seam for source-size limits.

use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_protocol::DependencyAction;
use gent_types::{
    Command, ProviderInstallProvenance, ProviderPromptProvisionCommandBinding,
    ProvisionedProviderInstallation, ReceiptStatus,
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
    provision_inner(provisioner, request, command, None)
}

pub(super) fn provision_prompt<I, P, V, R, B>(
    provisioner: &PrivateProviderProvisioner<I, P, V, R, B>,
    request: &PrivateProvisionRequest,
    command: &Command,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<PrivateProvisionResult, PrivateProvisionError>
where
    I: DependencyInstaller,
    P: PackageInstallPolicy,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
{
    provision_inner(provisioner, request, command, Some(binding))
}

fn provision_inner<I, P, V, R, B>(
    provisioner: &PrivateProviderProvisioner<I, P, V, R, B>,
    request: &PrivateProvisionRequest,
    command: &Command,
    binding: Option<&ProviderPromptProvisionCommandBinding>,
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
    if let Some(binding) = binding {
        prompt_binding_matches(command, request, binding)?;
    }
    let verifier = provisioner
        .verifier
        .as_ref()
        .ok_or(PrivateProvisionError::VerificationUnavailable)?;
    let npm = provisioner.runtime.rechecked_npm_prefix()?;
    let durable = provisioner
        .receipts
        .accepted_receipt(command)
        .map_err(PrivateProvisionError::ReceiptUnavailable)?;
    if durable != request.receipt || durable.status != ReceiptStatus::Accepted {
        return Err(PrivateProvisionError::ReceiptMismatch);
    }
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
    if let Some(binding) = binding {
        let expected = &binding.package;
        if package.provider != expected.provider
            || package.package_name != expected.package_name
            || package.version != expected.version
            || package.integrity != expected.integrity
            || package.package_policy_digest_sha256 != expected.package_policy_digest_sha256
        {
            return Err(PrivateProvisionError::PromptPackageMismatch);
        }
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
                    release_artifact_digest_sha256: String::new(),
                    receipt_fingerprint_sha256: command.receipt_fingerprint_sha256(),
                },
            },
        ))),
        Err(_) => Ok(PrivateProvisionResult::Ambiguous),
    }
}

fn prompt_binding_matches(
    command: &Command,
    request: &PrivateProvisionRequest,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Result<(), PrivateProvisionError> {
    let payload =
        serde_json::to_value(binding).map_err(|_| PrivateProvisionError::PromptBindingMismatch)?;
    (binding.is_valid()
        && command.kind == "providerPromptProvision"
        && command.payload == payload
        && binding.prompt.provider == request.provider.as_str()
        && binding.prompt.action
            == match request.action {
                DependencyAction::Install => "install",
                DependencyAction::Update => "update",
            }
        && binding.prompt.consent_granted == request.consent_granted
        && binding.prompt.reviewed_plan_digest == request.reviewed_plan_digest
        && binding.prompt.reviewed_plan_digest == binding.expected_reviewed_plan_digest)
        .then_some(())
        .ok_or(PrivateProvisionError::PromptBindingMismatch)
}
