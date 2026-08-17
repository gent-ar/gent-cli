//! Daemon-facing runtime port. Transport adapters depend only on this boundary.

use gent_protocol::{
    AgentChatIntentFrame, AttachmentFrame, DecisionRecoveryEvidence, DecisionSubmission,
    DependencyActionRequest, DependencyActionResult, DependencyPlan, DependencyPlanRequest,
    PermissionPolicyFrame, ProviderAuthFrame, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, ReviewedPlanFrame,
};
use gent_runtime::ConversationActivityRead;
use gent_types::{
    CapabilitySet, Command, ConversationContentCursor, ConversationContentPage,
    ConversationListItem, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventResume, HostStatus, OnboardingState, Receipt,
    RuntimeMaintenanceReport, RuntimeMaintenanceRequest, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckRequest,
};

pub(crate) trait RuntimeApi: Clone + Send + Sync + 'static {
    fn capabilities(&self) -> Result<CapabilitySet, String>;
    fn status(&self) -> Result<HostStatus, String>;
    fn submit(&self, command: Command) -> Result<Receipt, String>;
    fn resume_events(&self, cursor: u64) -> Result<EventResume, String>;
    fn doctor(&self) -> DoctorReport;
    fn onboarding(&self) -> OnboardingState {
        OnboardingState::from_doctor(&self.doctor())
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan;
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String>;
    /// Handles a capability-gated attachment transfer frame.
    fn attachment(&self, _: AttachmentFrame) -> Result<AttachmentFrame, String> {
        Err("attachments are unavailable for this runtime".into())
    }
    /// Reads a separately configured, signed cached release report.
    fn runtime_update_check(
        &self,
        _: RuntimeUpdateCheckRequest,
    ) -> Result<RuntimeUpdateCheckReport, String> {
        Err("runtime update checks are observer-disabled".into())
    }
    /// Reads one separately enabled durable update-maintenance attempt.
    fn runtime_maintenance(
        &self,
        _: RuntimeMaintenanceRequest,
    ) -> Result<RuntimeMaintenanceReport, String> {
        Err("runtime maintenance is observer-disabled".into())
    }
    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String>;
    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String>;
    fn start_public_run(&self, request: PublicRunStartRequest)
    -> Result<PublicRunResponse, String>;
    fn resume_public_run(
        &self,
        request: PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, String>;
    fn interrupt_public_run(
        &self,
        request: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String>;
    /// Handles a finite authority-gated agent-chat exchange.
    ///
    /// The transport validates that every response is correlated to `request`. Observer
    /// implementations keep this default and therefore cannot accept chat commands merely by
    /// negotiating the protocol capability.
    fn agent_chat_intent(
        &self,
        _: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, String> {
        Err("agent chat is unavailable while gentd is observer-disabled".into())
    }
    /// Reads or appends one local, provider-neutral permission-policy revision.
    fn permission_policy(&self, _: PermissionPolicyFrame) -> Result<PermissionPolicyFrame, String> {
        Err("permission policy is unavailable for this runtime".into())
    }
    /// Handles a secret-free provider-auth frame only in a future authority composition.
    ///
    /// The observer default refuses before any provider discovery, browser launch, or login
    /// process can be reached.
    fn provider_auth(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("provider authentication is unavailable while gentd is observer-disabled".into())
    }
    /// Handles reviewed-plan reads and user decisions only in a future authority composition.
    fn reviewed_plan(&self, _: ReviewedPlanFrame) -> Result<ReviewedPlanFrame, String> {
        Err("reviewed plans are unavailable while gentd is observer-disabled".into())
    }
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String>;
    fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
        Err("conversation discovery is unavailable for this runtime".into())
    }
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String>;
    /// Returns activity only for a future authority-gated runtime composition.
    fn conversation_activity(
        &self,
        _: &str,
        _: &str,
        _: u64,
    ) -> Result<ConversationActivityRead, String> {
        Err("conversation activity is unavailable for this runtime".into())
    }
    fn conversation_content(
        &self,
        _: &str,
        _: Option<ConversationContentCursor>,
        _: u16,
    ) -> Result<ConversationContentPage, String> {
        Err("conversation content is unavailable for this runtime".into())
    }
}
