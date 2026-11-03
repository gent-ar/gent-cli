//! Attach gateway lifetime to the actual contained process.
use super::{
    Arc, ProcessLauncher, ProviderGatewayLease, SandboxedLauncher, SandboxedProviderLaunch,
};

pub struct GatewayBoundLauncher<S> {
    pub(super) launcher: SandboxedLauncher<S>,
    pub(super) lease: Arc<ProviderGatewayLease>,
    #[cfg(any(windows, target_os = "linux"))]
    pub(super) mcp_control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
}
#[cfg(any(windows, target_os = "linux"))]
impl<S> GatewayBoundLauncher<S> {
    pub(crate) fn with_mcp_control(
        mut self,
        control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    ) -> Self {
        self.mcp_control = control;
        self
    }
}

impl<S> std::fmt::Debug for GatewayBoundLauncher<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayBoundLauncher")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

impl<S: SandboxedProviderLaunch + 'static> ProcessLauncher for GatewayBoundLauncher<S> {
    type Process = super::super::bound_process::GatewayBoundProcess<S::Process>;

    fn validate_permission_binding(
        &self,
        binding: &gent_types::SandboxPermissionBinding,
    ) -> Result<(), gent_drivers::SupervisorError> {
        if !self.lease.current()
            || binding.conversation_id() != self.lease.fence.conversation_id
            || binding.run_id() != self.lease.fence.run_id
            || binding.policy() != &self.lease.fence.policy
            || binding.host_epoch() != self.lease.fence.host_epoch
        {
            return Err(gent_drivers::SupervisorError::Launch(
                "provider gateway authority binding changed".into(),
            ));
        }
        Ok(())
    }

    fn launch(
        &self,
        launch: &gent_drivers::supervisor::ProviderLaunch,
    ) -> Result<Self::Process, gent_drivers::SupervisorError> {
        if !self.lease.current() || !self.lease.fence.matches(launch) {
            return Err(gent_drivers::SupervisorError::Launch(
                "provider gateway run binding changed".into(),
            ));
        }
        #[cfg(any(windows, target_os = "linux"))]
        let launch = &self.lease.prepare_mcp(launch, self.mcp_control.as_ref())?;
        let raw = match self.launcher.launch(launch) {
            Ok(process) => process,
            Err(error) => {
                super::super::bound_process::retain_unsettled(&self.lease);
                return Err(error);
            }
        };
        let process = super::super::bound_process::GatewayBoundProcess {
            process: Arc::new(raw),
            lease: self.lease.clone(),
        };
        #[cfg(any(windows, target_os = "linux"))]
        if self.lease.publish_mcp(&process.process).is_err() {
            drop(process);
            return Err(gent_drivers::SupervisorError::Launch(
                "host MCP process evidence is unavailable".into(),
            ));
        }
        if !self.lease.current() {
            drop(process);
            return Err(gent_drivers::SupervisorError::Launch(
                "provider gateway run binding changed".into(),
            ));
        }
        Ok(process)
    }
}
