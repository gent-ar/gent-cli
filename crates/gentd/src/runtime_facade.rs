//! Daemon composition of observer and explicitly approved durable-chat services.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gent_protocol::{AGENT_CHAT_INTENTS_CAPABILITY, REVIEWED_PLAN_CAPABILITY};
use gent_runtime::catalog::validate_observed_capabilities;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AgentChatReadService, AgentChatSelectionSwitchAuthority,
    AgentChatSelectionSwitchService, AttachmentService, Coordinator, DependencyActionService,
    GoalAuthority, GoalService, OrchestrationAuthority, OrchestrationService,
    ReviewedPlanAuthority, ReviewedPlanService, RuntimeMaintenanceAuthority,
    RuntimeMaintenanceService, TurnFollowService,
};
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::CapabilitySet;

use crate::compatibility_assessment::CompatibilityAssessment;
use crate::dependency_actions::ObserverDependencyExecutor;
use crate::dependency_catalog::DependencyCatalog;
use crate::public_runs::{DaemonPublicRuns, observer_service};
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;

#[path = "runtime_facade_authority.rs"]
mod authority;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeFacade {
    agent_chat_conversations: AgentChatConversationService<SqliteLedger>,
    agent_chat_prompts: AgentChatPromptService<SqliteLedger>,
    agent_chat_switches: AgentChatSelectionSwitchService<SqliteLedger>,
    agent_chat_reads: Option<AgentChatReadService<SqliteLedger>>,
    turn_follow_source: Option<SqliteLedger>,
    ordinary_prompt_wake: Option<
        Arc<Mutex<crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    >,
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

/// The one already-open durable state shared by future daemon compositions.
///
/// Opening this state prepares only current local storage and live capability validation.
/// It does not discover or launch a provider, and does not select any authority profile.
#[derive(Debug)]
pub(crate) struct DaemonCompositionState {
    data_dir: PathBuf,
    ledger: SqliteLedger,
    coordinator: Coordinator<SqliteLedger>,
    compatibility: CompatibilityAssessment,
}

#[allow(dead_code)] // Future approved composition reads these state components before facade build.
impl DaemonCompositionState {
    /// Opens the single Gent ledger and coordinator after rejecting capability drift.
    ///
    /// # Errors
    /// Returns an error when observed capabilities drift or the local durable state cannot open.
    pub(crate) fn open(
        data_dir: &Path,
        observed_capabilities: &CapabilitySet,
        compatibility: CompatibilityAssessment,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let capabilities = validate_observed_capabilities(observed_capabilities)?;
        let ledger = SqliteLedger::open(data_dir.join("gent.db"))?;
        crate::permission_workspace::ensure(&ledger, data_dir)?;
        let coordinator = Coordinator::new(ledger.clone(), capabilities);
        Ok(Self {
            data_dir: data_dir.into(),
            ledger,
            coordinator,
            compatibility,
        })
    }

    /// Returns the original immutable daemon data directory.
    #[must_use]
    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Returns the already-open Gent-owned ledger; no other component opens a replacement ledger.
    #[must_use]
    pub(crate) fn ledger(&self) -> &SqliteLedger {
        &self.ledger
    }

    /// Returns the coordinator bound to this state and its live capability declaration.
    #[must_use]
    pub(crate) fn coordinator(&self) -> &Coordinator<SqliteLedger> {
        &self.coordinator
    }

    /// Returns the compatibility assessment injected by the daemon composition root.
    #[must_use]
    pub(crate) fn compatibility(&self) -> &CompatibilityAssessment {
        &self.compatibility
    }
}

/// Builds the current observer-only daemon composition.
///
/// # Errors
/// Returns an error when capabilities drift or required local storage cannot open.
#[cfg(test)]
pub(crate) fn build_runtime(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    build_runtime_with_update_checks(data_dir, observed_capabilities, compatibility, None)
}

/// Builds the daemon with explicitly configured read-only update checks and durable-chat reads.
///
/// # Errors
/// Returns an error when capabilities drift or required local storage cannot open.
pub(crate) fn build_runtime_with_update_checks(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    let state = DaemonCompositionState::open(data_dir, observed_capabilities, compatibility)?;
    RuntimeFacade::from_state(state, runtime_update_checks)
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
        Self::from_state_inner(state, runtime_update_checks, false, None)
    }

    fn from_state_inner(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
        turn_follow_enabled: bool,
        ordinary_prompt_wake: Option<
            Arc<
                Mutex<
                    crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter<SqliteLedger>,
                >,
            >,
        >,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let DaemonCompositionState {
            data_dir,
            ledger,
            coordinator,
            compatibility,
        } = state;
        let capabilities = coordinator.status()?.capabilities;
        let agent_chat_enabled = capabilities
            .0
            .iter()
            .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY);
        let reviewed_plan_enabled = capabilities
            .0
            .iter()
            .any(|capability| capability == REVIEWED_PLAN_CAPABILITY);
        let maintenance_enabled = capabilities
            .0
            .iter()
            .any(|capability| capability == gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY);
        let attachments = AttachmentService::new(
            ledger.clone(),
            FileAttachmentBlobs::open(data_dir.join("attachments"))?,
        );
        let turn_follow_source = turn_follow_enabled.then(|| ledger.clone());
        Ok(Self {
            agent_chat_conversations: AgentChatConversationService::new(
                ledger.clone(),
                chat_authority(agent_chat_enabled),
            ),
            agent_chat_prompts: AgentChatPromptService::new(
                ledger.clone(),
                prompt_authority(agent_chat_enabled),
            ),
            agent_chat_switches: AgentChatSelectionSwitchService::new(
                ledger.clone(),
                switch_authority(agent_chat_enabled),
            ),
            agent_chat_reads: agent_chat_enabled.then(|| AgentChatReadService::new(ledger.clone())),
            turn_follow_source,
            ordinary_prompt_wake,
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
            dependencies: DependencyCatalog::with_private_prefix(
                compatibility,
                data_dir.join("providers").join("npm-global"),
            ),
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
