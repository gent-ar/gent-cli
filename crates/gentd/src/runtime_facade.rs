//! Daemon composition of observer and explicitly approved durable-chat services.

use std::sync::Arc;

use crate::dependency_actions::ObserverDependencyExecutor;
use crate::dependency_catalog::DependencyCatalog;
use crate::prompt_provider_provision_boundary::PromptProviderProvisionPort;
use crate::provider_readiness_boundary::ProviderReadinessPort;
use crate::public_runs::{DaemonPublicRuns, observer_service};
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AgentChatReadService, AgentChatSelectionGate,
    AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchService, AllowAnyAgentChatSelection,
    AttachmentService, Coordinator, DependencyActionService, GoalAuthority, GoalService,
    OrchestrationAuthority, OrchestrationService, ReviewedPlanAuthority, ReviewedPlanService,
    RuntimeMaintenanceAuthority, RuntimeMaintenanceService, TurnFollowService,
};
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::CapabilitySet;

type RuntimeSelectionGate = Arc<dyn AgentChatSelectionGate>;

#[path = "runtime_facade_authority.rs"]
mod authority;
#[path = "runtime_facade_chat_reads.rs"]
mod chat_reads;
#[path = "runtime_facade_composition.rs"]
mod composition;

#[cfg(test)]
pub(crate) use composition::build_runtime;
pub(crate) use composition::{DaemonCompositionState, build_runtime_with_update_checks};

#[derive(Clone)]
pub(crate) struct RuntimeFacade {
    agent_chat_conversations: AgentChatConversationService<SqliteLedger, RuntimeSelectionGate>,
    agent_chat_prompts: AgentChatPromptService<SqliteLedger>,
    agent_chat_switches: AgentChatSelectionSwitchService<SqliteLedger, RuntimeSelectionGate>,
    agent_chat_reads: Option<AgentChatReadService<SqliteLedger>>,
    provider_readiness: Option<Arc<dyn ProviderReadinessPort>>,
    prompt_provider_provision: Option<Arc<dyn PromptProviderProvisionPort>>,
    turn_follow_source: Option<SqliteLedger>,
    ordinary_prompt_ingress:
        Option<crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress<SqliteLedger>>,
    goals: GoalService<SqliteLedger>,
    reviewed_plans: ReviewedPlanService<SqliteLedger>,
    orchestration: OrchestrationService<SqliteLedger>,
    runtime_maintenance: RuntimeMaintenanceService<SqliteLedger>,
    attachments: AttachmentService<SqliteLedger, FileAttachmentBlobs>,
    coordinator: Coordinator<SqliteLedger>,
    dependencies: DependencyCatalog,
    dependency_actions: DependencyActionService<SqliteLedger, ObserverDependencyExecutor>,
    public_runs: DaemonPublicRuns,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
}

impl RuntimeFacade {
    /// Builds the current facade from one already-open daemon composition state.
    ///
    /// # Errors
    /// Returns an error when the durable attachment store cannot open.
    pub(crate) fn from_state(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::from_state_inner(
            state,
            runtime_update_checks,
            None,
            None,
            None,
            Arc::new(AllowAnyAgentChatSelection),
        )
    }

    fn from_state_inner(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
        ordinary_prompt_ingress: Option<
            crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress<SqliteLedger>,
        >,
        provider_readiness: Option<Arc<dyn ProviderReadinessPort>>,
        prompt_provider_provision: Option<Arc<dyn PromptProviderProvisionPort>>,
        selection_gate: RuntimeSelectionGate,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let DaemonCompositionState {
            data_dir,
            ledger,
            coordinator,
            capability_profile,
            compatibility,
        } = state;
        if capability_profile.provider_readiness_enabled() && provider_readiness.is_none() {
            return Err(
                "provider readiness requires an explicit private authority composition".into(),
            );
        }
        if !capability_profile.provider_readiness_enabled() && provider_readiness.is_some() {
            return Err(
                "provider readiness authority requires its typed capability profile".into(),
            );
        }
        if capability_profile.prompt_provider_provision_enabled()
            && prompt_provider_provision.is_none()
        {
            return Err(
                "prompt provider provision requires an explicit private authority composition"
                    .into(),
            );
        }
        if !capability_profile.prompt_provider_provision_enabled()
            && prompt_provider_provision.is_some()
        {
            return Err(
                "prompt provider provision authority requires its typed capability profile".into(),
            );
        }
        let agent_chat_enabled = capability_profile.agent_chat_enabled();
        let reviewed_plan_enabled = capability_profile.reviewed_plans_enabled();
        let maintenance_enabled = capability_profile.runtime_maintenance_enabled();
        let attachments = AttachmentService::new(
            ledger.clone(),
            FileAttachmentBlobs::open(data_dir.join("attachments"))?,
        );
        let turn_follow_source = capability_profile
            .turn_follow_enabled()
            .then(|| ledger.clone());
        let dependencies = DependencyCatalog::with_private_prefix(
            compatibility.clone(),
            data_dir.join("providers").join("npm-global"),
        );
        Ok(Self {
            agent_chat_conversations: AgentChatConversationService::with_selection_gate(
                ledger.clone(),
                chat_authority(agent_chat_enabled),
                Arc::clone(&selection_gate),
            ),
            agent_chat_prompts: AgentChatPromptService::new(
                ledger.clone(),
                prompt_authority(agent_chat_enabled),
            ),
            agent_chat_switches: AgentChatSelectionSwitchService::with_selection_gate(
                ledger.clone(),
                switch_authority(agent_chat_enabled),
                selection_gate,
            ),
            agent_chat_reads: agent_chat_enabled.then(|| AgentChatReadService::new(ledger.clone())),
            provider_readiness,
            prompt_provider_provision,
            turn_follow_source,
            ordinary_prompt_ingress,
            goals: GoalService::new(ledger.clone(), goal_authority(agent_chat_enabled)),
            reviewed_plans: ReviewedPlanService::new(
                ledger.clone(),
                reviewed_plan_authority(reviewed_plan_enabled),
            ),
            orchestration: OrchestrationService::new(
                ledger.clone(),
                orchestration_authority(agent_chat_enabled),
            ),
            runtime_maintenance: RuntimeMaintenanceService::new(
                ledger.clone(),
                maintenance_authority(maintenance_enabled),
            ),
            public_runs: observer_service(coordinator.clone(), compatibility.clone()),
            runtime_update_checks,
            attachments,
            coordinator,
            dependencies,
            dependency_actions: DependencyActionService::new(ledger, ObserverDependencyExecutor),
        })
    }
}

fn chat_authority(enabled: bool) -> AgentChatConversationAuthority {
    if enabled {
        AgentChatConversationAuthority::Approved
    } else {
        AgentChatConversationAuthority::Observer
    }
}

fn prompt_authority(enabled: bool) -> AgentChatPromptAuthority {
    if enabled {
        AgentChatPromptAuthority::Approved
    } else {
        AgentChatPromptAuthority::Observer
    }
}

fn switch_authority(enabled: bool) -> AgentChatSelectionSwitchAuthority {
    if enabled {
        AgentChatSelectionSwitchAuthority::Approved
    } else {
        AgentChatSelectionSwitchAuthority::Observer
    }
}

fn maintenance_authority(enabled: bool) -> RuntimeMaintenanceAuthority {
    if enabled {
        RuntimeMaintenanceAuthority::Approved
    } else {
        RuntimeMaintenanceAuthority::Observer
    }
}

fn goal_authority(enabled: bool) -> GoalAuthority {
    if enabled {
        GoalAuthority::Approved
    } else {
        GoalAuthority::Observer
    }
}

fn orchestration_authority(enabled: bool) -> OrchestrationAuthority {
    if enabled {
        OrchestrationAuthority::Approved
    } else {
        OrchestrationAuthority::Observer
    }
}

fn reviewed_plan_authority(enabled: bool) -> ReviewedPlanAuthority {
    if enabled {
        ReviewedPlanAuthority::Approved
    } else {
        ReviewedPlanAuthority::Observer
    }
}

include!("runtime_facade_api.rs");
