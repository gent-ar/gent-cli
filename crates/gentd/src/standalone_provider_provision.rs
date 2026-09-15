use std::sync::Arc;

use gent_drivers::installer::SystemDependencyInstaller;

use crate::{
    authority_clock::SystemAuthorityClock,
    dependency_catalog::DependencyCatalog,
    ordinary_authority_release::{OrdinaryAuthorityReleaseError, VerifiedOrdinaryAuthorityRelease},
    private_provider_provisioning::PrivateProviderProvisioner,
    private_provider_provisioning_sqlite::SqliteProvisionReceiptReader,
    private_provider_verifier::PrivatePrefixProvisionedProviderVerifier,
    prompt_provider_provision_boundary::{
        PromptProviderProvisionBoundary, PromptProviderProvisionPort,
    },
    runtime_facade::DaemonCompositionState,
    standalone_authority_release::StandaloneAuthorityRelease,
};

pub(crate) fn compose(
    state: &DaemonCompositionState,
    release: &StandaloneAuthorityRelease,
    verified: &VerifiedOrdinaryAuthorityRelease,
) -> Result<Arc<dyn PromptProviderProvisionPort>, String> {
    let runtime = release
        .runtime()
        .ok_or_else(|| OrdinaryAuthorityReleaseError::RuntimeUnverified.to_string())?;
    let installed_verifier = PrivatePrefixProvisionedProviderVerifier::system(
        runtime
            .rechecked_lock()
            .map_err(|error| error.to_string())?,
    );
    let provisioner = PrivateProviderProvisioner::with_compatibility(
        runtime.clone(),
        SystemDependencyInstaller,
        release.clone(),
        Some(installed_verifier),
        SqliteProvisionReceiptReader::new(state.ledger().clone()),
        verified.compatibility(),
        Some(release.provision_config()),
    );
    Ok(Arc::new(PromptProviderProvisionBoundary::new(
        state.ledger().clone(),
        DependencyCatalog::with_private_prefix(
            verified.compatibility(),
            state.data_dir().join("providers").join("npm-global"),
        ),
        release.clone(),
        provisioner,
        SystemAuthorityClock,
        verified.artifact_digest_sha256().into(),
    )))
}
