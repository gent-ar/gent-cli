/// One concrete handler eligible for an authority profile's wire advertisement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapabilityFeature {
    AgentChat,
    ConversationActivity,
    AgentChatPermissions,
    AgentChatProjection,
    TurnFollow,
    ReviewedPlans,
    ProviderReadiness,
    ProviderAuth,
    PromptProviderProvision,
    RuntimeUpdateCheck,
    RuntimeMaintenance,
    LocalModels,
    PromptTemplates,
    WorkspaceDocuments,
    WorkspaceGit,
}

/// Concrete runtime handlers eligible for one authority profile's wire advertisement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeCapabilityProfile {
    features: Vec<RuntimeCapabilityFeature>,
}

impl RuntimeCapabilityProfile {
    /// Builds a stable deduplicated profile from concrete handler observations.
    #[must_use]
    pub fn new(features: impl IntoIterator<Item = RuntimeCapabilityFeature>) -> Self {
        let mut unique = Vec::new();
        for feature in features {
            if !unique.contains(&feature) {
                unique.push(feature);
            }
        }
        Self { features: unique }
    }

    pub(super) fn has(&self, feature: RuntimeCapabilityFeature) -> bool {
        self.features.contains(&feature)
    }

    #[must_use]
    pub fn agent_chat_enabled(&self) -> bool {
        self.has(RuntimeCapabilityFeature::AgentChat)
    }

    #[must_use]
    pub fn agent_chat_permissions_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::AgentChatPermissions)
    }

    #[must_use]
    pub fn conversation_activity_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::ConversationActivity)
    }

    #[must_use]
    pub fn agent_chat_projection_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::AgentChatProjection)
    }

    #[must_use]
    pub fn turn_follow_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::TurnFollow)
    }

    #[must_use]
    pub fn reviewed_plans_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::ReviewedPlans)
    }

    #[must_use]
    pub fn provider_readiness_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::ProviderReadiness)
    }

    #[must_use]
    pub fn provider_auth_enabled(&self) -> bool {
        self.agent_chat_enabled() && self.has(RuntimeCapabilityFeature::ProviderAuth)
    }

    #[must_use]
    pub fn prompt_provider_provision_enabled(&self) -> bool {
        self.provider_readiness_enabled()
            && self.has(RuntimeCapabilityFeature::PromptProviderProvision)
    }

    #[must_use]
    pub fn runtime_update_check_enabled(&self) -> bool {
        self.has(RuntimeCapabilityFeature::RuntimeUpdateCheck)
    }

    #[must_use]
    pub fn runtime_maintenance_enabled(&self) -> bool {
        self.has(RuntimeCapabilityFeature::RuntimeMaintenance)
    }

    #[must_use]
    pub fn local_models_enabled(&self) -> bool {
        self.has(RuntimeCapabilityFeature::LocalModels)
    }

    pub fn prompt_templates_enabled(&self) -> bool {
        self.has(RuntimeCapabilityFeature::PromptTemplates)
    }
}
