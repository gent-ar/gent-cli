//! A capability is issued only after verified installation resources AND a real
//! settled kernel probe. It is not deserializable and cannot attest a user run.
use crate::{
    linux_provider_kernel_probe::{self, KernelProbeSeal},
    linux_provider_resources::LinuxProviderResources,
    permission_backend_registry::VerifiedProviderFactoryResources,
    provider_executables::ProviderExecutables,
};
use gent_types::{AgentChatProvider, RunVersionLock, SandboxPermissionBinding};
use std::sync::Arc;

pub(crate) struct LinuxFactoryAuthorization {
    resources: Arc<LinuxProviderResources>,
    probe: KernelProbeSeal,
}
impl std::fmt::Debug for LinuxFactoryAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxFactoryAuthorization")
            .finish_non_exhaustive()
    }
}
impl LinuxFactoryAuthorization {
    pub(crate) fn prepare(
        providers: &ProviderExecutables,
        provider: AgentChatProvider,
        launch: Option<&RunVersionLock>,
    ) -> Result<Self, String> {
        if provider != AgentChatProvider::Claude {
            return Err("The native Linux provider contract is currently Claude-only".into());
        }
        let installed = providers
            .launch_lock(provider)
            .map_err(|_| "Managed Linux provider installation is unavailable")?;
        let lock = launch.unwrap_or(&installed);
        let resources = Arc::new(providers.linux_resources(provider, lock)?);
        let probe = linux_provider_kernel_probe::run(resources.clone())?;
        let result = Self { resources, probe };
        result.require(provider)?;
        Ok(result)
    }
    pub(crate) fn require(&self, provider: AgentChatProvider) -> Result<(), String> {
        if provider != AgentChatProvider::Claude || !self.probe.matches(&self.resources) {
            return Err(
                "Native Linux capability no longer matches its installation/service".into(),
            );
        }
        self.resources.recheck()
    }
    pub(crate) fn require_launch(
        &self,
        provider: AgentChatProvider,
        binding: &SandboxPermissionBinding,
        lock: &RunVersionLock,
    ) -> Result<(), String> {
        self.require(provider)?;
        if !binding.policy().requires_sandbox()
            || self.resources.executable() != lock
            || lock.provider != "claude"
        {
            return Err("Native Linux capability does not match the contained user launch".into());
        }
        Ok(())
    }
    pub(crate) fn resources(&self) -> &LinuxProviderResources {
        &self.resources
    }
}
