//! Linux-only managed-lease composition; no credential capture or platform fallback.
use super::{
    Arc, Coordinator, EgressError, GatewayBoundLauncher, Path, ProviderGatewayLease,
    RunVersionLock, SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxResourceGrant,
    SandboxResourceLimits, SandboxedLauncher, SqliteLedger, claude_credential,
};
use gent_linux_sandbox::{GatewayAuthority, LinuxMxcLauncher, RuntimePin};
use gent_types::{SandboxCpuSemantics, SandboxPermissionBinding};
use std::path::PathBuf;

struct RetainedLinuxLease(Arc<ProviderGatewayLease>);
impl gent_linux_sandbox::gateway_bridge::LiveGatewayLease for RetainedLinuxLease {
    fn current(&self) -> bool {
        self.0.current()
    }
}

/// Installation authority is provided by reviewed runtime metadata, never user argv.
/// Baseline grants stay explicit in `resources`; the backend rejects missing coverage.
pub(crate) struct LinuxRuntimeDeployment {
    pub delegated_parent: PathBuf,
    pub helper_lock: RunVersionLock,
    pub network_runtime: RuntimePin,
    pub cpu_semantics: SandboxCpuSemantics,
}

impl ProviderGatewayLease {
    /// Compose the actual existing lease/fence/broker with the owned native backend.
    /// A caller cannot turn an endpoint or serialized descriptor into this lease.
    pub(crate) fn launcher_linux(
        self: &Arc<Self>,
        deployment: LinuxRuntimeDeployment,
        launch: &gent_drivers::supervisor::ProviderLaunch,
        limits: SandboxResourceLimits,
        mut resources: Vec<SandboxResourceGrant>,
    ) -> Result<GatewayBoundLauncher<LinuxMxcLauncher>, EgressError> {
        if !self.current() || self.fence.lock.provider != "claude" || !self.fence.matches(launch) {
            return Err(EgressError::Configuration);
        }
        if !matches!(
            deployment.cpu_semantics,
            SandboxCpuSemantics::MonitoredTotal { .. }
        ) {
            return Err(EgressError::Configuration);
        }
        deployment
            .network_runtime
            .verify()
            .map_err(|_| EgressError::Configuration)?;
        let parent = gent_linux_sandbox::delegation::current_service_parent("controller")
            .map_err(|_| EgressError::Configuration)?;
        if parent != deployment.delegated_parent {
            return Err(EgressError::Configuration);
        }
        if gent_drivers::lock::rechecked_identity(&deployment.helper_lock)
            .map_err(|_| EgressError::Configuration)?
            != deployment.helper_lock
        {
            return Err(EgressError::Configuration);
        }
        // The selected public gate must execute inside the contained namespace.
        // Installation authority approves that exact image, not its directory.
        resources.push(SandboxResourceGrant::PrivateStateReadWrite(
            self.private_state.root().to_owned(),
        ));
        super::super::linux_resources::include_helper_image(
            &mut resources,
            &deployment.helper_lock,
        )
        .map_err(|_| EgressError::Configuration)?;
        let policy = SandboxLaunchPolicy::new(
            vec![],
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256: self.broker.policy_digest().into(),
            },
            limits,
        )
        .map_err(|_| EgressError::Configuration)?
        .with_resources(resources)
        .with_private_state(self.private_state.clone())
        .with_permission_binding(
            SandboxPermissionBinding::new(
                self.fence.conversation_id.clone(),
                self.fence.run_id.clone(),
                self.fence.policy.clone(),
                self.fence.host_epoch,
            )
            .map_err(|_| EgressError::Configuration)?,
        )
        .with_cpu_semantics(deployment.cpu_semantics)
        .map_err(|_| EgressError::Configuration)?;
        // Preflight the exact durable workspace and every already-approved OS grant.
        let profile = policy
            .profile_for_workspace(
                Path::new(&self.fence.workspace.canonical_path),
                launch.workspace_access,
            )
            .map_err(|_| EgressError::Configuration)?;
        gent_linux_sandbox::mxc_baseline::validate(&profile)
            .map_err(|_| EgressError::Configuration)?;
        let backend = LinuxMxcLauncher {
            delegated_parent: deployment.delegated_parent,
            helper_lock: deployment.helper_lock,
            gateway_authority: Some(GatewayAuthority {
                lease: Arc::new(RetainedLinuxLease(self.clone())),
                runtime: deployment.network_runtime,
            }),
        };
        let launcher = SandboxedLauncher::new(policy, backend)
            .with_gateway_for_run(self.fence.run_id.clone(), self.broker.gateway_binding()?)
            .map_err(|_| EgressError::Configuration)?;
        if !self.current() {
            return Err(EgressError::Configuration);
        }
        Ok(GatewayBoundLauncher {
            launcher,
            lease: self.clone(),
            mcp_control: None,
        })
    }
}

/// A reviewed installation supplies these exact locks. This type does not discover
/// or self-authorize an executable from PATH, and does not enable provider admission.
/// Release trust and compatibility must have been verified by the installation owner.
pub(crate) struct LinuxCredentialDeployment {
    pub provider_lock: RunVersionLock,
    pub helper_lock: RunVersionLock,
    pub delegated_parent: PathBuf,
}

impl ProviderGatewayLease {
    pub(crate) fn claude_managed_linux(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
        probe_root: &Path,
        deployment: LinuxCredentialDeployment,
    ) -> Result<Arc<Self>, EgressError> {
        if lock != deployment.provider_lock
            || lock.provider != "claude"
            || gent_linux_sandbox::delegation::current_service_parent("controller")
                .map_err(|_| EgressError::Authentication)?
                != deployment.delegated_parent
        {
            return Err(EgressError::Authentication);
        }
        let home = std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .filter(|v| v.is_absolute() && v.is_dir())
            .ok_or(EgressError::Authentication)?;
        let mut environment = std::collections::BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), home.to_string_lossy().into_owned()),
        ]);
        if let Ok(config) = std::env::var("CLAUDE_CONFIG_DIR") {
            if !Path::new(&config).is_absolute() {
                return Err(EgressError::Authentication);
            }
            environment.insert("CLAUDE_CONFIG_DIR".into(), config);
        }
        Self::claude_using(
            ledger,
            coordinator,
            conversation_id,
            run_id,
            lock,
            probe_root,
            |lock| {
                claude_credential::resolve_using(
                    lock,
                    probe_root,
                    environment,
                    |lock, cwd, args, environment, timeout| {
                        let mut environment = environment.clone();
                        environment.insert(
                            "TMPDIR".into(),
                            cwd.parent()
                                .ok_or_else(super::super::probe_process::ProbeFailure::known)?
                                .to_string_lossy()
                                .into_owned(),
                        );
                        let result = gent_linux_sandbox::host_probe::run_classified(
                            &deployment.delegated_parent,
                            &deployment.helper_lock,
                            lock,
                            cwd,
                            args,
                            &environment,
                            timeout,
                        )
                        .map_err(|failure| {
                            if failure.scope_known() {
                                super::super::probe_process::ProbeFailure::known()
                            } else {
                                super::super::probe_process::ProbeFailure::retained(failure)
                            }
                        })?;
                        Ok(super::super::probe_process::ProbeOutput {
                            stdout: result.stdout,
                            stderr_empty: result.stderr_empty,
                            success: true,
                        })
                    },
                )
            },
        )
    }
}

impl ProviderGatewayLease {
    /// Installation owner supplies the whole exact selected server lock set.
    /// This is neither runtime discovery nor permission admission.
    pub(crate) fn configure_mcp_linux(
        &self,
        deployment: super::super::mcp_owner::LinuxMcpDeployment,
    ) -> Result<(), EgressError> {
        if !self.current() {
            return Err(EgressError::Configuration);
        }
        let owner = self.mcp.lock().map_err(|_| EgressError::Configuration)?;
        let mut slot = self
            .mcp_deployment
            .lock()
            .map_err(|_| EgressError::Configuration)?;
        if owner.is_some() || slot.is_some() {
            return Err(EgressError::Configuration);
        }
        *slot = Some(deployment);
        Ok(())
    }
}
