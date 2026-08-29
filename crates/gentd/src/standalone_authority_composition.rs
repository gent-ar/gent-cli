use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gent_drivers::process::SystemLauncher;
use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::{LocalModelDescriptor, LocalModelInstallState};
use gent_runtime::{AgentChatReadService, Coordinator};
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use crate::{
    claude_standalone_authority::{
        StandaloneClaudeConfig, StandaloneClaudeError, compose_standalone_claude,
    },
    codex_standalone_authority::{
        StandaloneCodexConfig, StandaloneCodexError, compose_standalone_codex,
    },
    local_model_catalog::LocalModelCatalog,
    local_model_provisioning::{LocalModelDownloadPlan, LocalModelProvisioner, ModelInstallState},
    ordinary_lifecycle_cadence::{
        OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair_with_standalone_models,
    },
    ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ordinary_lifecycle_router::{
        OrdinaryLifecycleHost, OrdinaryProviderHost, OrdinaryPublicLifecycleRouter,
    },
    provider_lifecycle_host::ProviderLifecycleHost,
    runtime_facade::DaemonCompositionState,
    standalone_claurst_runtime_factory::StandaloneClaurstBridge,
};

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneAuthorityConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) claude_executable: Option<PathBuf>,
    pub(crate) codex_executable: Option<PathBuf>,
    pub(crate) mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
}

struct LazyStandaloneProviderHost {
    provider: AgentChatProvider,
    data_dir: PathBuf,
    executable: Option<PathBuf>,
    ledger: SqliteLedger,
    coordinator: Coordinator<SqliteLedger>,
    coordinator_id: String,
    host_epoch: gent_types::HostEpoch,
    mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
    host: Option<Box<dyn OrdinaryLifecycleHost>>,
}

impl LazyStandaloneProviderHost {
    fn start(&mut self) -> Result<(), ()> {
        if self.host.is_some() {
            return Ok(());
        }
        let executable = self
            .executable
            .clone()
            .map_or_else(
                || crate::standalone_provider_setup::ensure_provider(&self.data_dir, self.provider),
                Ok,
            )
            .map_err(|_| ())?;
        self.start_with(executable)
    }

    fn start_existing(&mut self) -> Result<(), ()> {
        if self.host.is_some() {
            return Ok(());
        }
        let executable = self
            .executable
            .clone()
            .filter(|path| path.is_file())
            .or_else(|| {
                crate::standalone_provider_setup::installed_provider_executable(
                    &self.data_dir,
                    self.provider,
                )
            });
        executable.map_or(Ok(()), |executable| self.start_with(executable))
    }

    fn start_with(&mut self, executable: PathBuf) -> Result<(), ()> {
        let launcher = SystemLauncher::new(STREAM_CAPTURE_BYTES);
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
                            executable,
                            mcp_config: self.mcp_config.as_ref().map(|config| config.path().into()),
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
                            executable,
                            mcp_servers: self
                                .mcp_config
                                .as_ref()
                                .map(crate::standalone_mcp_config::StandaloneMcpConfig::codex_servers)
                                .transpose()
                                .map_err(|_| ())?,
                            mcp_config: self.mcp_config.as_ref().map(|config| config.path().into()),
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
        // A restart must not strand a prompt that was already accepted. Only
        // durable pending work may cause this lazy host to provision a
        // provider; an idle startup remains side-effect free.
        if self.host.is_none()
            && self
                .ledger
                .has_pending_agent_chat_prompt_dispatch(self.provider)
                .map_err(|_| ())?
        {
            self.start()?;
        } else {
            self.start_existing()?;
        }
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

#[derive(Clone, Debug)]
pub(crate) struct StandaloneAuthorityRuntime {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    control: OrdinaryLifecycleControl,
    prompt_ingress: OrdinaryPromptIngress<SqliteLedger>,
    cadence: OrdinaryLifecycleCadence<SqliteLedger>,
    claurst_models: StandaloneClaurstModels,
    claurst_runtime: Arc<Mutex<Option<RetainedClaurstRuntime>>>,
    claurst_side_question_bridge: Arc<Mutex<Option<StandaloneClaurstBridge>>>,
    ledger: SqliteLedger,
    coordinator_id: String,
    host_epoch: gent_types::HostEpoch,
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneClaurstModels {
    pub(crate) provisioner: LocalModelProvisioner,
    active_downloads: Arc<Mutex<HashSet<String>>>,
}

type SystemClaurstRuntime = crate::claurst_standalone_owner::ClaurstStandaloneRuntime<
    crate::claurst_local_runtime_owner::SystemLocalRuntimeProcess,
    crate::claurst_local_runtime_owner::SystemClaurstAcpStdio,
>;

struct RetainedClaurstRuntime(SystemClaurstRuntime);

impl std::fmt::Debug for RetainedClaurstRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RetainedClaurstRuntime(..)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LocalModelDownloadStart {
    Ready {
        size_bytes: u64,
    },
    Download {
        plan: LocalModelDownloadPlan,
        resumed_bytes: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StandaloneClaurstModelStatus {
    DownloadRequired {
        plan: LocalModelDownloadPlan,
        downloaded_bytes: u64,
    },
    Ready {
        model_path: PathBuf,
    },
}

impl StandaloneClaurstModels {
    pub(crate) fn from_data_dir(
        data_dir: &std::path::Path,
    ) -> Result<Self, StandaloneAuthorityError> {
        let catalog =
            LocalModelCatalog::shipped().map_err(|_| StandaloneAuthorityError::ClaurstCatalog)?;
        Ok(Self {
            provisioner: LocalModelProvisioner::new(data_dir, catalog),
            active_downloads: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub(crate) fn assess(
        &self,
        model_id: &str,
    ) -> Result<StandaloneClaurstModelStatus, StandaloneAuthorityError> {
        let plan = self
            .provisioner
            .plan(model_id)
            .map_err(|_| StandaloneAuthorityError::UnknownClaurstModel)?;
        match self
            .provisioner
            .state(model_id)
            .map_err(|_| StandaloneAuthorityError::ClaurstStorage)?
        {
            ModelInstallState::NotInstalled => Ok(StandaloneClaurstModelStatus::DownloadRequired {
                plan,
                downloaded_bytes: 0,
            }),
            ModelInstallState::Downloading { downloaded_bytes } => {
                Ok(StandaloneClaurstModelStatus::DownloadRequired {
                    plan,
                    downloaded_bytes,
                })
            }
            ModelInstallState::Ready { path } => {
                Ok(StandaloneClaurstModelStatus::Ready { model_path: path })
            }
        }
    }

    pub(crate) fn catalogue(&self) -> Vec<LocalModelDescriptor> {
        self.provisioner
            .catalogue()
            .models()
            .iter()
            .map(|model| LocalModelDescriptor {
                id: model.id.clone(),
                label: model.label.clone(),
                size_bytes: model.size_bytes,
            })
            .collect()
    }

    pub(crate) fn install_state(
        &self,
        model_id: &str,
    ) -> Result<LocalModelInstallState, StandaloneAuthorityError> {
        let plan = self
            .provisioner
            .plan(model_id)
            .map_err(|_| StandaloneAuthorityError::UnknownClaurstModel)?;
        match self
            .provisioner
            .state(model_id)
            .map_err(|_| StandaloneAuthorityError::ClaurstStorage)?
        {
            ModelInstallState::NotInstalled => Ok(LocalModelInstallState::NotInstalled),
            ModelInstallState::Downloading { downloaded_bytes } => {
                Ok(LocalModelInstallState::Downloading {
                    downloaded_bytes,
                    total_bytes: plan.expected_bytes,
                })
            }
            ModelInstallState::Ready { .. } => Ok(LocalModelInstallState::Ready {
                size_bytes: plan.expected_bytes,
            }),
        }
    }

    pub(crate) fn begin_download(
        &self,
        model_id: &str,
    ) -> Result<LocalModelDownloadStart, StandaloneAuthorityError> {
        let plan = self
            .provisioner
            .plan(model_id)
            .map_err(|_| StandaloneAuthorityError::UnknownClaurstModel)?;
        match self
            .provisioner
            .state(model_id)
            .map_err(|_| StandaloneAuthorityError::ClaurstStorage)?
        {
            ModelInstallState::Ready { .. } => Ok(LocalModelDownloadStart::Ready {
                size_bytes: plan.expected_bytes,
            }),
            ModelInstallState::NotInstalled => self.claim_download(plan, 0),
            ModelInstallState::Downloading { downloaded_bytes } => {
                self.claim_download(plan, downloaded_bytes)
            }
        }
    }

    pub(crate) fn finish_download(&self, model_id: &str) {
        if let Ok(mut active) = self.active_downloads.lock() {
            active.remove(model_id);
        }
    }

    pub(crate) fn download_active(&self, model_id: &str) -> bool {
        self.active_downloads
            .lock()
            .map(|active| active.contains(model_id))
            .unwrap_or(false)
    }

    fn claim_download(
        &self,
        plan: LocalModelDownloadPlan,
        resumed_bytes: u64,
    ) -> Result<LocalModelDownloadStart, StandaloneAuthorityError> {
        let mut active = self
            .active_downloads
            .lock()
            .map_err(|_| StandaloneAuthorityError::ClaurstStorage)?;
        if !active.insert(plan.model_id.clone()) {
            return Err(StandaloneAuthorityError::ClaurstDownloadInProgress);
        }
        Ok(LocalModelDownloadStart::Download {
            plan,
            resumed_bytes,
        })
    }
}

impl StandaloneAuthorityRuntime {
    #[must_use]
    pub(crate) fn agent_chat_permission_port(
        &self,
    ) -> std::sync::Arc<dyn crate::agent_chat_permission_api::AgentChatPermissionPort> {
        std::sync::Arc::new(
            crate::agent_chat_permission_api::StandaloneAgentChatPermissionPort::new(
                self.ledger.clone(),
                self.prompt_ingress(),
            ),
        )
    }

    #[must_use]
    pub(crate) fn router(&self) -> Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>> {
        Arc::clone(&self.router)
    }

    #[must_use]
    pub(crate) fn prompt_ingress(&self) -> OrdinaryPromptIngress<SqliteLedger> {
        self.prompt_ingress.clone()
    }

    #[must_use]
    pub(crate) fn lifecycle_control(&self) -> OrdinaryLifecycleControl {
        self.control.clone()
    }

    pub(crate) fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), String> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    pub(crate) fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(|_| "Claude permission owner is unavailable".to_owned())
    }

    pub(crate) fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_codex_permission(run_id, request_id, decision, answers)
            .map_err(|_| "Codex permission owner is unavailable".to_owned())
    }

    pub(crate) async fn respond_claurst_permission(
        &self,
        response: gent_types::PermissionDecisionResponse,
    ) -> Result<(), String> {
        self.prompt_ingress
            .respond_claurst_permission(response)
            .await
    }

    #[must_use]
    pub(crate) fn claurst_models(&self) -> &StandaloneClaurstModels {
        &self.claurst_models
    }

    pub(crate) async fn run_cadence(&self) -> Result<(), String> {
        self.cadence.clone().run().await
    }

    pub(crate) async fn wait_until_ready(&self) -> Result<(), String> {
        self.control
            .wait_until_ready()
            .await
            .map_err(|_| "standalone lifecycle shut down before it became ready".to_owned())
    }

    pub(crate) async fn attach_claurst_bridge<B>(&self, bridge: B) -> Result<(), String>
    where
        B: gent_ports::PrivateClaurstBridge + std::fmt::Debug + Send + 'static,
    {
        self.prompt_ingress
            .attach_async_claurst(Box::new(
                crate::claurst_prompt_lifecycle::ClaurstPromptLifecycle::new(
                    self.ledger.clone(),
                    bridge,
                    self.coordinator_id.clone(),
                    self.host_epoch,
                ),
            ))
            .await
    }

    pub(crate) async fn attach_lazy_claurst_runtime(
        &self,
        config: Option<crate::standalone_claurst_runtime_factory::StandaloneClaurstRuntimeConfig>,
    ) -> Result<(), String> {
        let factory = std::sync::Arc::new(
            crate::standalone_claurst_runtime_factory::StandaloneClaurstRuntimeFactory::new(
                self.ledger.clone(),
                self.claurst_models.clone(),
                config,
            ),
        );
        if let Ok(mut bridge) = self.claurst_side_question_bridge.lock() {
            *bridge = Some(factory.bridge());
        }
        self.prompt_ingress
            .attach_async_claurst(Box::new(
                crate::claurst_prompt_lifecycle::ClaurstPromptLifecycle::new_with_runtime(
                    self.ledger.clone(),
                    factory.bridge(),
                    factory,
                    self.coordinator_id.clone(),
                    self.host_epoch,
                ),
            ))
            .await
    }

    /// The bridge side questions use to run on the currently attached local Claurst runtime,
    /// once `attach_lazy_claurst_runtime` has run. `None` before then, or when Claurst is
    /// attached through `attach_claurst_bridge`/`start_local_claurst` instead.
    #[must_use]
    pub(crate) fn claurst_side_question_bridge(&self) -> Option<StandaloneClaurstBridge> {
        self.claurst_side_question_bridge
            .lock()
            .ok()
            .and_then(|bridge| bridge.clone())
    }

    pub(crate) async fn start_local_claurst(
        &self,
        model_id: String,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: PathBuf,
    ) -> Result<(), String> {
        if self
            .claurst_runtime
            .lock()
            .map_err(|_| "standalone Claurst runtime lock is unavailable".to_owned())?
            .is_some()
        {
            return Err("a standalone Claurst runtime is already active".into());
        }
        let readiness = crate::claurst_local_readiness::ClaurstLocalReadinessService::new(
            self.claurst_models.provisioner.clone(),
        );
        let runtime = tokio::task::spawn_blocking(move || {
            crate::claurst_standalone_owner::ClaurstStandaloneOwner::new(
                readiness,
                crate::claurst_local_runtime_owner::SystemPrivateSettingsStore,
                crate::claurst_local_runtime_owner::SystemClaurstStandaloneLauncher,
                crate::claurst_local_runtime_owner::HttpLlamaServerReadiness::default(),
            )
            .start(&model_id, request, &workspace)
        })
        .await
        .map_err(|_| "local Claurst startup worker stopped unexpectedly".to_owned())?
        .map_err(|error| error.to_string())?;
        let bridge = crate::claurst_acp_bridge::ClaurstBridgeHandle::new(runtime.bridge());
        if let Err(error) = self.attach_claurst_bridge(bridge).await {
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err(error);
        }
        let mut retained = self
            .claurst_runtime
            .lock()
            .map_err(|_| "standalone Claurst runtime lock is unavailable".to_owned())?;
        if retained.is_some() {
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err("a standalone Claurst runtime is already active".into());
        }
        *retained = Some(RetainedClaurstRuntime(runtime));
        Ok(())
    }

    pub(crate) fn drive_once(&self) -> Result<bool, StandaloneAuthorityError> {
        self.router
            .lock()
            .map_err(|_| StandaloneAuthorityError::RouterUnavailable)?
            .drive_once()
            .map_err(|_| StandaloneAuthorityError::RouterUnavailable)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum StandaloneAuthorityError {
    #[error("standalone authority cannot inspect daemon ownership state")]
    StateUnavailable,
    #[error(transparent)]
    Claude(#[from] StandaloneClaudeError),
    #[error(transparent)]
    Codex(#[from] StandaloneCodexError),
    #[error("the shipped Claurst model catalog is invalid")]
    ClaurstCatalog,
    #[error("selected Claurst model is not in Gent's curated catalog")]
    UnknownClaurstModel,
    #[error("could not inspect Gent-owned Claurst model storage")]
    ClaurstStorage,
    #[error("a curated Claurst model download is already active")]
    ClaurstDownloadInProgress,
    #[error("standalone lifecycle router is unavailable")]
    RouterUnavailable,
}

pub(crate) fn compose_standalone_authority(
    state: &DaemonCompositionState,
    config: &StandaloneAuthorityConfig,
) -> Result<StandaloneAuthorityRuntime, StandaloneAuthorityError> {
    let (coordinator_id, host_epoch) = owner(state)?;
    let claurst_models = StandaloneClaurstModels::from_data_dir(state.data_dir())?;
    let hosts: Vec<Box<dyn OrdinaryLifecycleHost>> = vec![
        Box::new(LazyStandaloneProviderHost {
            provider: AgentChatProvider::Claude,
            data_dir: config.data_dir.clone(),
            executable: config.claude_executable.clone(),
            ledger: state.ledger().clone(),
            coordinator: state.coordinator().clone(),
            coordinator_id: coordinator_id.clone(),
            host_epoch,
            host: None,
            mcp_config: config.mcp_config.clone(),
        }),
        Box::new(LazyStandaloneProviderHost {
            provider: AgentChatProvider::Codex,
            data_dir: config.data_dir.clone(),
            executable: config.codex_executable.clone(),
            ledger: state.ledger().clone(),
            coordinator: state.coordinator().clone(),
            coordinator_id: coordinator_id.clone(),
            host_epoch,
            host: None,
            mcp_config: config.mcp_config.clone(),
        }),
    ];
    let router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(state.ledger().clone()),
        hosts,
    )
    .map_err(|_| StandaloneAuthorityError::RouterUnavailable)?;
    let router = Arc::new(Mutex::new(router));
    let (control, prompt_ingress, cadence) = pair_with_standalone_models(
        Arc::clone(&router),
        state.ledger().clone(),
        host_epoch,
        claurst_models.clone(),
    );
    Ok(StandaloneAuthorityRuntime {
        router,
        control,
        prompt_ingress,
        cadence,
        claurst_models,
        claurst_runtime: Arc::new(Mutex::new(None)),
        claurst_side_question_bridge: Arc::new(Mutex::new(None)),
        ledger: state.ledger().clone(),
        coordinator_id,
        host_epoch,
    })
}

fn owner(
    state: &DaemonCompositionState,
) -> Result<(String, gent_types::HostEpoch), StandaloneAuthorityError> {
    let epoch = state
        .coordinator()
        .status()
        .map_err(|_| StandaloneAuthorityError::StateUnavailable)?
        .host_epoch;
    Ok((format!("gentd-{}", epoch.0), epoch))
}

#[cfg(test)]
#[path = "standalone_authority_composition_tests.rs"]
mod tests;
