use std::sync::Arc;

use gent_drivers::installer::SystemDependencyInstaller;

use crate::{
    authority_clock::SystemAuthorityClock,
    dependency_catalog::DependencyCatalog,
    private_provider_provisioning::PrivateProviderProvisioner,
    private_provider_provisioning_sqlite::SqliteProvisionReceiptReader,
    private_provider_verifier::PrivatePrefixProvisionedProviderVerifier,
    prompt_provider_provision_boundary::{
        PromptProviderProvisionBoundary, PromptProviderProvisionPort,
    },
    runtime_facade::DaemonCompositionState,
    standalone_authority_release::StandaloneAuthorityRelease,
    startup,
};

pub(crate) fn compose(
    state: &DaemonCompositionState,
    release: &StandaloneAuthorityRelease,
) -> Result<Arc<dyn PromptProviderProvisionPort>, String> {
    let verified = release
        .load(startup::unix_seconds())
        .map_err(|error| error.to_string())?;
    let verifier = PrivatePrefixProvisionedProviderVerifier::system(
        release
            .runtime()
            .rechecked_lock()
            .map_err(|error| error.to_string())?,
    );
    let provisioner = PrivateProviderProvisioner::with_compatibility(
        release.runtime().clone(),
        SystemDependencyInstaller,
        release.clone(),
        Some(verifier),
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
