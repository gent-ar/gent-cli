use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use crate::{
    claude_standalone_authority::StandaloneClaudeError,
    codex_standalone_authority::StandaloneCodexError,
    ordinary_lifecycle_cadence::{
        OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair_with_standalone_models,
    },
    ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter},
    runtime_facade::DaemonCompositionState,
    standalone_claurst_runtime_factory::StandaloneClaurstBridge,
};

#[path = "standalone_authority_composition_claurst_models.rs"]
mod claurst_models;
#[path = "standalone_authority_composition_claurst_runtime.rs"]
mod claurst_runtime;
#[path = "standalone_authority_composition_provider_host.rs"]
mod provider_host;

pub(crate) use claurst_models::StandaloneClaurstModels;
use claurst_runtime::RetainedClaurstRuntime;
use provider_host::LazyStandaloneProviderHost;

#[derive(Clone, Debug)]
pub(crate) struct StandaloneAuthorityConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) executables: crate::provider_executables::ProviderExecutables,
    pub(crate) mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
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
    provider_readiness:
        Arc<crate::standalone_provider_readiness::StandaloneProviderReadinessAuthority>,
    ledger: SqliteLedger,
    coordinator_id: String,
    host_epoch: gent_types::HostEpoch,
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
    pub(crate) fn provider_readiness_port(
        &self,
    ) -> Arc<dyn crate::provider_readiness_boundary::ProviderReadinessPort> {
        self.provider_readiness.clone()
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
        let pursuit = tokio::spawn(
            crate::goal_pursuit_host::GoalPursuitHost::new(
                self.ledger.clone(),
                self.prompt_ingress.clone(),
                self.host_epoch,
            )
            .run(self.control.clone()),
        );
        let result = self.cadence.clone().run().await;
        pursuit.abort();
        result
    }

    pub(crate) async fn wait_until_ready(&self) -> Result<(), String> {
        self.control
            .wait_until_ready()
            .await
            .map_err(|_| "standalone lifecycle shut down before it became ready".to_owned())
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
    #[error("standalone lifecycle router is unavailable")]
    RouterUnavailable,
}

pub(crate) fn compose_standalone_authority(
    state: &DaemonCompositionState,
    config: &StandaloneAuthorityConfig,
) -> Result<StandaloneAuthorityRuntime, StandaloneAuthorityError> {
    let (coordinator_id, host_epoch) = owner(state)?;
    let claurst_models = StandaloneClaurstModels::from_data_dir(
        state.data_dir(),
        crate::local_model_events::LocalModelEvents::new(state.ledger().clone(), host_epoch),
    )?;
    let provider_readiness = Arc::new(
        crate::standalone_provider_readiness::StandaloneProviderReadinessAuthority::new(
            state.ledger().clone(),
            config.executables.clone(),
            claurst_models.clone(),
        ),
    );
    let lazy_host = |provider| -> Box<dyn OrdinaryLifecycleHost> {
        Box::new(LazyStandaloneProviderHost::new(
            provider,
            state,
            config,
            coordinator_id.clone(),
            host_epoch,
            provider_readiness.clone(),
        ))
    };
    let hosts = vec![
        lazy_host(AgentChatProvider::Claude),
        lazy_host(AgentChatProvider::Codex),
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
        provider_readiness.clone(),
    );
    Ok(StandaloneAuthorityRuntime {
        router,
        control,
        prompt_ingress,
        cadence,
        claurst_models,
        claurst_runtime: Arc::new(Mutex::new(None)),
        claurst_side_question_bridge: Arc::new(Mutex::new(None)),
        provider_readiness,
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
