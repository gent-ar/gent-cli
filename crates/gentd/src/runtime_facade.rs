//! Daemon composition of the observer runtime and its infrastructure adapters.

use gent_drivers::installer::SystemDependencyInstaller;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AttachmentFrame, DecisionRecoveryEvidence,
    DecisionSubmission, DependencyActionRequest, DependencyActionResult, DependencyPlan,
    DependencyPlanRequest,
};
use gent_runtime::catalog::validate_observed_capabilities;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AttachmentService, Coordinator, DependencyActionService,
};
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::{
    CapabilitySet, Command, ConversationContentCursor, ConversationContentPage, ConversationStatus,
    ConversationTimeline, DecisionCommand, DecisionSettlement, DoctorReport, EventResume,
    HostStatus, Receipt,
};

use crate::agent_chat_api;
use crate::api;
use crate::attachment_api;
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::decision_mapping;
use crate::dependency_actions::SystemDependencyExecutor;
use crate::dependency_catalog::DependencyCatalog;
use crate::public_runs::{DaemonPublicRuns, observer_service};
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeFacade {
    agent_chat_conversations: AgentChatConversationService<SqliteLedger>,
    agent_chat_prompts: AgentChatPromptService<SqliteLedger>,
    attachments: AttachmentService<SqliteLedger, FileAttachmentBlobs>,
    coordinator: Coordinator<SqliteLedger>,
    dependencies: DependencyCatalog,
    dependency_actions:
        DependencyActionService<SqliteLedger, SystemDependencyExecutor<SystemDependencyInstaller>>,
    public_runs: DaemonPublicRuns,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
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

/// Builds the daemon with an optional explicitly configured read-only update checker.
///
/// # Errors
/// Returns an error when capabilities drift or required local storage cannot open.
pub(crate) fn build_runtime_with_update_checks(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    let capabilities = validate_observed_capabilities(observed_capabilities)?;
    let agent_chat_enabled = capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY);
    let ledger = SqliteLedger::open(data_dir.join("gent.db"))?;
    let attachments = AttachmentService::new(
        ledger.clone(),
        FileAttachmentBlobs::open(data_dir.join("attachments"))?,
    );
    let coordinator = Coordinator::new(ledger.clone(), capabilities);
    coordinator.persist_capability_catalog()?;
    Ok(RuntimeFacade {
        agent_chat_conversations: AgentChatConversationService::new(
            ledger.clone(),
            if agent_chat_enabled {
                AgentChatConversationAuthority::Approved
            } else {
                AgentChatConversationAuthority::Observer
            },
        ),
        agent_chat_prompts: AgentChatPromptService::new(
            ledger.clone(),
            if agent_chat_enabled {
                AgentChatPromptAuthority::Approved
            } else {
                AgentChatPromptAuthority::Observer
            },
        ),
        public_runs: observer_service(coordinator.clone(), compatibility.clone()),
        runtime_update_checks,
        attachments,
        coordinator,
        dependencies: DependencyCatalog::with_compatibility(compatibility),
        dependency_actions: DependencyActionService::new(
            ledger,
            SystemDependencyExecutor::new(SystemDependencyInstaller),
        ),
    })
}

impl api::RuntimeApi for RuntimeFacade {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        self.coordinator
            .status()
            .map(|status| status.capabilities)
            .map_err(|error| error.to_string())
    }

    fn status(&self) -> Result<HostStatus, String> {
        self.coordinator.status().map_err(|error| error.to_string())
    }

    fn submit(&self, command: Command) -> Result<Receipt, String> {
        self.coordinator
            .submit(&command)
            .map_err(|error| error.to_string())
    }

    fn resume_events(&self, cursor: u64) -> Result<EventResume, String> {
        self.coordinator
            .resume_events(cursor)
            .map_err(|error| error.to_string())
    }

    fn agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        agent_chat_api::exchange(
            &self.agent_chat_conversations,
            &self.agent_chat_prompts,
            host_epoch,
            frame,
        )
    }

    fn doctor(&self) -> DoctorReport {
        self.dependencies.doctor()
    }

    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        self.dependencies.plan(request)
    }

    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String> {
        let plan = self.dependencies.plan(DependencyPlanRequest {
            provider: request.provider,
            action: request.action,
        });
        self.dependency_actions
            .execute(&request, &plan)
            .map_err(|error| error.to_string())
    }

    fn attachment(&self, frame: AttachmentFrame) -> Result<AttachmentFrame, String> {
        attachment_api::handle(&self.attachments, frame)
    }

    fn runtime_update_check(
        &self,
        request: gent_types::RuntimeUpdateCheckRequest,
    ) -> Result<gent_types::RuntimeUpdateCheckReport, String> {
        self.runtime_update_checks
            .as_ref()
            .map(|checks| checks.check(request, crate::startup::unix_seconds()))
            .ok_or_else(|| "runtime update checks are observer-disabled".into())
    }

    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String> {
        self.coordinator
            .submit_decision(command)
            .map(decision_mapping::submission)
            .map_err(|error| error.to_string())
    }

    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String> {
        self.coordinator
            .apply_decision_evidence(&decision_id, decision_mapping::recovery(evidence))
            .map_err(|error| error.to_string())
    }

    fn start_public_run(
        &self,
        request: gent_protocol::PublicRunStartRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .start(request)
            .map_err(|error| error.to_string())
    }

    fn resume_public_run(
        &self,
        request: gent_protocol::PublicRunResumeRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .resume(request)
            .map_err(|error| error.to_string())
    }

    fn interrupt_public_run(
        &self,
        request: gent_protocol::PublicRunInterruptRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .interrupt(request)
            .map_err(|error| error.to_string())
    }

    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String> {
        self.coordinator
            .conversation_status(conversation_id)
            .map_err(|error| error.to_string())
    }

    fn conversations(&self) -> Result<Vec<gent_types::ConversationListItem>, String> {
        self.coordinator
            .conversations()
            .map_err(|error| error.to_string())
    }

    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String> {
        self.coordinator
            .conversation_timeline(conversation_id)
            .map_err(|error| error.to_string())
    }

    fn conversation_content(
        &self,
        conversation_id: &str,
        before: Option<ConversationContentCursor>,
        limit: u16,
    ) -> Result<ConversationContentPage, String> {
        self.coordinator
            .conversation_content(conversation_id, before.as_ref(), limit)
            .map_err(|error| error.to_string())
    }
}
