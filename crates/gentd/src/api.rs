//! Daemon-facing runtime port. Transport adapters depend only on this boundary.

use gent_protocol::{
    AttachmentFrame, DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest,
    DependencyActionResult, DependencyPlan, DependencyPlanRequest, PublicRunInterruptRequest,
    PublicRunResponse, PublicRunResumeRequest, PublicRunStartRequest,
};
use gent_types::{
    CapabilitySet, Command, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventResume, HostStatus, OnboardingState, Receipt,
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
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String>;
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String>;
}
