//! Normal Linux broad-mode factory. Installed authority, a real settled kernel
//! probe and a fresh run binding are required; host fallback is never selected.
use super::{ProviderLaunch, RoutedProviderProcess, SandboxProviderLauncher, SupervisorError};
use gent_drivers::ProcessLauncher;
impl SandboxProviderLauncher {
    pub(super) fn linux_authority(
        &self,
        provider: gent_types::AgentChatProvider,
        lock: Option<&gent_types::RunVersionLock>,
    ) -> Result<crate::linux_provider_factory::LinuxFactoryAuthorization, SupervisorError> {
        let providers = self.managed_providers.as_ref().ok_or_else(|| {
            SupervisorError::Launch("Managed Linux installation authority is unavailable".into())
        })?;
        crate::linux_provider_factory::LinuxFactoryAuthorization::prepare(providers, provider, lock)
            .map_err(SupervisorError::Launch)
    }
    pub(super) fn contained_linux(
        &self,
        launch: &ProviderLaunch,
    ) -> Result<RoutedProviderProcess, SupervisorError> {
        let binding = launch
            .permission_binding
            .as_ref()
            .ok_or_else(|| SupervisorError::Launch("Provider run authority missing".into()))?;
        let provider = self.current(binding)?;
        if provider != gent_types::AgentChatProvider::Claude || launch.provider != "claude" {
            return Err(SupervisorError::Launch(
                "Native Linux contained provider is not yet verified".into(),
            ));
        }
        let authority = self.linux_authority(provider, Some(&launch.lock))?;
        crate::permission_backend_registry::require_verified_linux_launch(
            provider,
            binding,
            &launch.lock,
            &authority,
        )
        .map_err(SupervisorError::Launch)?;
        let resources = authority.resources();
        let mcp = resources
            .mcp_for_launch(launch)
            .map_err(SupervisorError::Launch)?;
        std::fs::create_dir_all(&self.state_root)
            .map_err(|_| SupervisorError::Launch("Provider state unavailable".into()))?;
        let lease = crate::provider_egress::ProviderGatewayLease::claude_managed_linux(
            self.ledger.clone(),
            self.coordinator.clone(),
            binding,
            launch.lock.clone(),
            &self.state_root,
            resources.credentials(),
        )
        .map_err(|_| {
            SupervisorError::Launch(
                "Native Claude credential capture/gateway preparation failed".into(),
            )
        })?;
        if let Some(deployment) = mcp {
            lease.configure_mcp_linux(deployment).map_err(|_| {
                SupervisorError::Launch("Native selected MCP deployment rejected".into())
            })?;
        }
        let launcher = lease
            .launcher_linux(
                resources
                    .runtime_deployment()
                    .map_err(SupervisorError::Launch)?,
                launch,
                gent_types::SandboxResourceLimits {
                    max_processes: 64,
                    max_memory_bytes: 2 * 1024 * 1024 * 1024,
                    max_cpu_time_ms: 300_000,
                },
                resources.grants(),
            )
            .map_err(|_| SupervisorError::Launch("Native Linux sandbox profile rejected".into()))?
            .with_mcp_control(self.lifecycle_control.clone());
        lease.retain_resource_handles(
            resources
                .retained_handles()
                .map_err(SupervisorError::Launch)?,
        );
        self.current(binding)?;
        launcher.launch(launch).map(RoutedProviderProcess::new)
    }
}
