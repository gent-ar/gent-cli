use std::path::PathBuf;

use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use super::StandaloneAuthorityConfig;
use crate::{
    claude_standalone_authority::{StandaloneClaudeConfig, compose_standalone_claude},
    codex_standalone_authority::{StandaloneCodexConfig, compose_standalone_codex},
    ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryProviderHost},
    provider_lifecycle_host::ProviderLifecycleHost,
    runtime_facade::DaemonCompositionState,
    standalone_mcp_config::StandaloneMcpConfig,
    standalone_provider_readiness::StandalonePublicProviderReadinessPort,
};

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

pub(super) struct LazyStandaloneProviderHost {
    provider: AgentChatProvider,
    data_dir: PathBuf,
    executables: crate::provider_executables::ProviderExecutables,
    ledger: SqliteLedger,
    coordinator: Coordinator<SqliteLedger>,
    coordinator_id: String,
    host_epoch: gent_types::HostEpoch,
    mcp_config: Option<StandaloneMcpConfig>,
    readiness: StandalonePublicProviderReadinessPort,
    host: Option<Box<dyn OrdinaryLifecycleHost>>,
}

impl LazyStandaloneProviderHost {
    pub(super) fn new(
        provider: AgentChatProvider,
        state: &DaemonCompositionState,
        config: &StandaloneAuthorityConfig,
        coordinator_id: String,
        host_epoch: gent_types::HostEpoch,
        readiness: StandalonePublicProviderReadinessPort,
    ) -> Self {
        Self {
            provider,
            data_dir: config.data_dir.clone(),
            executables: config.executables.clone(),
            ledger: state.ledger().clone(),
            coordinator: state.coordinator().clone(),
            coordinator_id,
            host_epoch,
            mcp_config: config.mcp_config.clone(),
            readiness,
            host: None,
        }
    }

    fn start(&mut self) -> Result<(), ()> {
        if self.host.is_some() {
            return Ok(());
        }
        if !self.readiness.is_ready(self.provider).map_err(|_| ())? {
            return Err(());
        }
        self.start_with()
    }

    fn start_existing(&mut self) -> Result<(), ()> {
        if self.host.is_some() {
            return Ok(());
        }
        if !self.readiness.is_ready(self.provider).map_err(|_| ())? {
            return Ok(());
        }
        if self.executables.executable(self.provider).is_none() {
            return Ok(());
        }
        self.start_with()
    }

    fn start_with(&mut self) -> Result<(), ()> {
        let launcher = crate::node_runtime_lock::standalone_provider_launcher(STREAM_CAPTURE_BYTES);
        let mcp_config = self.mcp_config.as_ref().map(|config| config.path().into());
        let host: Box<dyn OrdinaryLifecycleHost> = match self.provider {
            AgentChatProvider::Claude => Box::new(OrdinaryProviderHost::new(
                self.provider,
                ProviderLifecycleHost::new(
                    compose_standalone_claude(
                        self.ledger.clone(),
                        self.coordinator.clone(),
                        &StandaloneClaudeConfig {
                            data_dir: self.data_dir.clone(),
                            coordinator_id: self.coordinator_id.clone(),
                            host_epoch: self.host_epoch,
                            executables: self.executables.clone(),
                            mcp_config,
                        },
                        launcher,
                    )
                    .map_err(|_| ())?,
                ),
            )),
            AgentChatProvider::Codex => Box::new(OrdinaryProviderHost::new(
                self.provider,
                ProviderLifecycleHost::new(
                    compose_standalone_codex(
                        self.ledger.clone(),
                        self.coordinator.clone(),
                        &StandaloneCodexConfig {
                            data_dir: self.data_dir.clone(),
                            coordinator_id: self.coordinator_id.clone(),
                            host_epoch: self.host_epoch,
                            executables: self.executables.clone(),
                            mcp_servers: self
                                .mcp_config
                                .as_ref()
                                .map(StandaloneMcpConfig::codex_servers)
                                .transpose()
                                .map_err(|_| ())?,
                            mcp_config,
                        },
                        launcher,
                    )
                    .map_err(|_| ())?,
                ),
            )),
            AgentChatProvider::Claurst => return Err(()),
        };
        self.host = Some(host);
        Ok(())
    }
}

impl OrdinaryLifecycleHost for LazyStandaloneProviderHost {
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.start_existing()?;
        self.host
            .as_mut()
            .map_or(Ok(()), |host| host.arm_authority_recovery())
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.start()?;
        self.host.as_mut().ok_or(())?.wake()
    }

    fn drive(&mut self) -> Result<(), ()> {
        self.host.as_mut().map_or(Ok(()), |host| host.drive())
    }

    fn needs_drive(&self) -> bool {
        self.host.as_ref().is_some_and(|host| host.needs_drive())
    }

    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), ()> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        self.host
            .as_ref()
            .ok_or(())?
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
    }

    fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        self.host
            .as_ref()
            .ok_or(())?
            .respond_codex_permission(run_id, request_id, decision, answers)
    }

    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.host.as_mut().ok_or(())?.interrupt_run(run_id)
    }

    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        self.host
            .as_mut()
            .map_or(Ok(()), |host| host.begin_shutdown_after_recovery())
    }

    fn escalate_shutdown(&mut self) -> Result<(), ()> {
        self.host
            .as_mut()
            .map_or(Ok(()), |host| host.escalate_shutdown())
    }

    fn shutdown_complete(&self) -> bool {
        self.host
            .as_ref()
            .is_none_or(|host| host.shutdown_complete())
    }
}
