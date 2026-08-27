//! Explicit non-observer facade constructors kept outside the default composition.

use std::{collections::BTreeSet, sync::Arc};

use gent_runtime::{AgentChatSelectionGate, ProviderModeSelectionGate};
use gent_types::{AgentChatMode, AgentChatProvider};

use super::{DaemonCompositionState, RuntimeFacade};
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;

impl RuntimeFacade {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_state_with_standalone_authority(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
        prompt_ingress: crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress<
            gent_store::SqliteLedger,
        >,
        local_models: crate::standalone_authority_composition::StandaloneClaurstModels,
        mcp_server_count: u16,
        mcp_server_names: Vec<String>,
        agent_chat_side_question_runners: Option<
            crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources,
        >,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if !state.capability_profile().local_models_enabled() {
            return Err("standalone local models require their typed capability profile".into());
        }
        let permission_port = state
            .capability_profile()
            .agent_chat_permissions_enabled()
            .then(|| {
                std::sync::Arc::new(
                    crate::agent_chat_permission_api::StandaloneAgentChatPermissionPort::new(
                        state.ledger().clone(),
                        prompt_ingress.clone(),
                    ),
                )
                    as std::sync::Arc<dyn crate::agent_chat_permission_api::AgentChatPermissionPort>
            });
        let selection_gate = StandaloneSelectionGate::new(local_models.catalogue());
        Self::from_state_inner(
            state,
            runtime_update_checks,
            Some(prompt_ingress),
            None,
            Some(local_models),
            mcp_server_count,
            mcp_server_names,
            permission_port,
            Arc::new(selection_gate),
            agent_chat_side_question_runners,
        )
    }
}

#[derive(Debug)]
struct StandaloneSelectionGate {
    provider_modes: ProviderModeSelectionGate,
    claurst_models: BTreeSet<String>,
}

impl StandaloneSelectionGate {
    fn new(models: Vec<gent_protocol::LocalModelDescriptor>) -> Self {
        Self {
            provider_modes: ProviderModeSelectionGate::new(
                [
                    AgentChatProvider::Claude,
                    AgentChatProvider::Codex,
                    AgentChatProvider::Claurst,
                ],
                [
                    AgentChatMode::Ask,
                    AgentChatMode::Plan,
                    AgentChatMode::Agent,
                ],
            ),
            claurst_models: models.into_iter().map(|model| model.id).collect(),
        }
    }
}

impl AgentChatSelectionGate for StandaloneSelectionGate {
    fn allows(&self, selection: &gent_types::AgentChatSelection) -> bool {
        self.provider_modes.allows(selection)
            && (selection.provider != AgentChatProvider::Claurst
                || self.claurst_models.contains(&selection.model))
    }
}

#[cfg(test)]
#[path = "runtime_facade_standalone_selection_tests.rs"]
mod standalone_selection_tests;
