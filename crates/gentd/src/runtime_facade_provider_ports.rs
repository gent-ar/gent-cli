use gent_protocol::{PromptProviderProvisionFrame, ProviderAuthFrame, ProviderReadinessFrame};

use super::RuntimeFacade;

impl RuntimeFacade {
    pub(super) fn managed_dependency_plan(
        &self,
        request: gent_protocol::DependencyPlanRequest,
    ) -> gent_protocol::DependencyPlan {
        self.prompt_provider_provision
            .as_ref()
            .and_then(|port| port.managed_plan(request.clone()))
            .unwrap_or_else(|| self.dependencies.plan(request))
    }
    pub(super) fn managed_dependency_action(
        &self,
        request: gent_protocol::DependencyActionRequest,
    ) -> Result<gent_protocol::DependencyActionResult, String> {
        if let Some(result) = self
            .prompt_provider_provision
            .as_ref()
            .and_then(|port| port.managed_action(request.clone()))
        {
            return result;
        }
        let plan = self
            .dependencies
            .plan(gent_protocol::DependencyPlanRequest {
                provider: request.provider,
                action: request.action,
            });
        self.dependency_actions
            .execute(&request, &plan)
            .map_err(|e| e.to_string())
    }

    pub(super) fn exchange_permission_policy(
        &self,
        frame: gent_protocol::PermissionPolicyFrame,
    ) -> Result<gent_protocol::PermissionPolicyFrame, String> {
        crate::permission_policy_api::exchange(&self.coordinator, frame)
    }

    pub(super) fn exchange_provider_auth(
        &self,
        frame: ProviderAuthFrame,
    ) -> Result<ProviderAuthFrame, String> {
        self.provider_auth
            .as_ref()
            .ok_or_else(|| {
                "provider authentication is unavailable while gentd is observer-disabled".to_owned()
            })?
            .exchange(frame)
    }

    pub(super) fn assess_provider_readiness(
        &self,
        frame: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        self.provider_readiness
            .as_ref()
            .ok_or_else(|| "provider readiness is observer-disabled".to_owned())?
            .assess(frame)
    }

    pub(super) fn confirm_prompt_provider_provision(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        self.prompt_provider_provision
            .as_ref()
            .ok_or_else(|| "prompt provider provisioning is observer-disabled".to_owned())?
            .confirm(frame)
    }
}
