use gent_drivers::installer::DependencyInstaller;
use gent_ports::PackageInstallPolicy;
use gent_types::{Command, ProviderPromptProvisionCommandBinding};

use crate::private_provider_compatibility::ProvisionedProviderCompatibility;
use crate::private_provider_provisioning::{
    PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
    PrivateProvisionResult, ProvisionReceiptReader, ProvisionedProviderVerifier,
};

pub(crate) trait PromptProviderProvisionEffect: Clone + Send + Sync {
    fn provision_prompt(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError>;
}

impl<I, P, V, R, B> PromptProviderProvisionEffect for PrivateProviderProvisioner<I, P, V, R, B>
where
    I: DependencyInstaller + Clone + Send + Sync,
    P: PackageInstallPolicy + Clone + Send + Sync,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
{
    fn provision_prompt(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        self.provision_prompt_with_command(request, command, binding)
    }
}
