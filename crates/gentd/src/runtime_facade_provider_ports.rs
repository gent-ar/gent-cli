use gent_protocol::{PromptProviderProvisionFrame, ProviderAuthFrame, ProviderReadinessFrame};

use super::RuntimeFacade;

impl RuntimeFacade {
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
