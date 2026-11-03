use super::ManagedProviderActionPort;
use crate::prompt_provider_provision_boundary::PromptProviderProvisionPort;
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionResult, DependencyPlan,
    DependencyPlanRequest, PromptProviderProvisionFrame,
};
use std::sync::Arc;
pub(crate) struct ManagedProvisionPorts {
    pub(crate) prompt: Arc<dyn PromptProviderProvisionPort>,
    pub(crate) actions: Arc<dyn ManagedProviderActionPort>,
}
impl PromptProviderProvisionPort for ManagedProvisionPorts {
    fn confirm(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        self.prompt.confirm(frame)
    }
    fn managed_plan(&self, request: DependencyPlanRequest) -> Option<DependencyPlan> {
        (request.action == DependencyAction::Update).then(|| self.actions.plan(request))
    }
    fn managed_action(
        &self,
        request: DependencyActionRequest,
    ) -> Option<Result<DependencyActionResult, String>> {
        (request.action == DependencyAction::Update).then(|| self.actions.execute(request))
    }
}

/// Holds normal lifecycle admission across the complete external update effect and settlement.
pub(crate) fn with_operation(
    control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    dispatch: impl FnOnce() -> Option<Result<DependencyActionResult, String>>,
) -> Option<Result<DependencyActionResult, String>> {
    let Ok(_permit) = control.acquire_operation() else {
        return Some(Err(
            "managed provider update admission is unavailable".into()
        ));
    };
    dispatch()
}
