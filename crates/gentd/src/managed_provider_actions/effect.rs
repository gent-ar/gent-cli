use super::ReviewedPackage;
use crate::{
    private_provider_compatibility::ProvisionedProviderCompatibility,
    private_provider_provisioning::{
        PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
        PrivateProvisionResult, ProvisionReceiptReader, ProvisionedProviderVerifier,
    },
};
use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_types::Command;
pub(crate) trait ManagedProviderEffect: Clone + Send + Sync {
    fn provision_reviewed(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        review: &ReviewedPackage,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError>;
}
impl<I, P, V, R, B> ManagedProviderEffect for PrivateProviderProvisioner<I, P, V, R, B>
where
    I: DependencyInstaller + Clone + Send + Sync,
    P: PackageInstallPolicy + Clone + Send + Sync,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
{
    fn provision_reviewed(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        review: &ReviewedPackage,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        self.provision_reviewed_with_command(request, command, review)
    }
}
