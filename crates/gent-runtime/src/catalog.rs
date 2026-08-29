//! Capability reconciliation is pure: live declarations must match observed behavior.

use gent_protocol::{
    AGENT_CHAT_CHECKPOINT_CAPABILITY, AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY,
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_SESSIONS_CAPABILITY, AGENT_CHAT_SIDE_QUESTION_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY, ATTACHMENTS_CAPABILITY,
    AUTOMATIONS_CAPABILITY, CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_CONTENT_CAPABILITY,
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY, FORGE_CONNECTORS_CAPABILITY,
    GOAL_CAPABILITY, LOCAL_MODELS_CAPABILITY, ORCHESTRATION_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_READINESS_CAPABILITY, REVIEWED_PLAN_CAPABILITY,
    RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
};
use gent_types::CapabilitySet;

/// A capability the runtime may advertise after its transport proves a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapability {
    AgentChatIntents,
    Attachments,
    Decisions,
    EventStream,
    Events,
    HostEpoch,
    PermissionPolicies,
    Receipts,
}

impl RuntimeCapability {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::AgentChatIntents => AGENT_CHAT_INTENTS_CAPABILITY,
            Self::Attachments => ATTACHMENTS_CAPABILITY,
            Self::Decisions => "decisions",
            Self::EventStream => EVENT_STREAM_CAPABILITY,
            Self::Events => "events",
            Self::HostEpoch => "host-epoch",
            Self::PermissionPolicies => gent_protocol::PERMISSION_POLICY_CAPABILITY,
            Self::Receipts => "receipts",
        }
    }
}

const DECLARED: [RuntimeCapability; 7] = [
    RuntimeCapability::Attachments,
    RuntimeCapability::Decisions,
    RuntimeCapability::EventStream,
    RuntimeCapability::Events,
    RuntimeCapability::HostEpoch,
    RuntimeCapability::PermissionPolicies,
    RuntimeCapability::Receipts,
];

/// One concrete handler eligible for an authority profile's wire advertisement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapabilityFeature {
    AgentChat,
    ConversationActivity,
    AgentChatPermissions,
    TurnFollow,
    ReviewedPlans,
    ProviderReadiness,
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

    fn has(&self, feature: RuntimeCapabilityFeature) -> bool {
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

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CatalogError {
    #[error("declared capability is not observed: {0}")]
    DeclaredButUnavailable(String),
    #[error("observed capability is absent from the catalog: {0}")]
    UndeclaredObserved(String),
}

/// Detects catalog drift before a daemon advertises capability availability.
///
/// # Errors
/// Returns an error for any declared-but-unavailable or observed-but-undeclared capability.
pub fn reconcile(declared: &CapabilitySet, observed: &CapabilitySet) -> Result<(), CatalogError> {
    for capability in &declared.0 {
        if !observed.0.contains(capability) {
            return Err(CatalogError::DeclaredButUnavailable(capability.clone()));
        }
    }
    for capability in &observed.0 {
        if !declared.0.contains(capability) {
            return Err(CatalogError::UndeclaredObserved(capability.clone()));
        }
    }
    Ok(())
}

/// Returns the one runtime-owned catalog eligible for wire advertisement.
#[must_use]
pub fn declared_capabilities() -> CapabilitySet {
    declared_capabilities_with_agent_chat(false)
}

/// Returns the catalog for an explicitly approved durable agent-chat authority profile.
#[must_use]
pub fn declared_capabilities_with_agent_chat(agent_chat_enabled: bool) -> CapabilitySet {
    let profile = RuntimeCapabilityProfile::new(
        agent_chat_enabled.then_some(RuntimeCapabilityFeature::AgentChat),
    );
    declared_capabilities_with_profiles(&profile)
}

/// Returns the catalog for explicit authority profiles that have concrete handlers.
#[must_use]
pub fn declared_capabilities_with_profiles(profile: &RuntimeCapabilityProfile) -> CapabilitySet {
    let mut capabilities = capability_set(DECLARED);
    if profile.agent_chat_enabled() {
        capabilities
            .0
            .push(RuntimeCapability::AgentChatIntents.wire_name().into());
        capabilities
            .0
            .push(AGENT_CHAT_CONVERSATIONS_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_TRANSCRIPT_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_SESSIONS_CAPABILITY.to_owned());
        capabilities.0.push(GOAL_CAPABILITY.to_owned());
        capabilities.0.push(ORCHESTRATION_CAPABILITY.to_owned());
        capabilities.0.push(AUTOMATIONS_CAPABILITY.to_owned());
        capabilities.0.push(FORGE_CONNECTORS_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_CHECKPOINT_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_SIDE_QUESTION_CAPABILITY.to_owned());
        if profile.prompt_templates_enabled() {
            capabilities
                .0
                .push(gent_protocol::PROMPT_TEMPLATES_CAPABILITY.to_owned());
        }
        if profile.has(RuntimeCapabilityFeature::WorkspaceDocuments) {
            capabilities
                .0
                .push(gent_protocol::WORKSPACE_DOCUMENTS_CAPABILITY.to_owned());
        }
        if profile.has(RuntimeCapabilityFeature::WorkspaceGit) {
            capabilities
                .0
                .push(gent_protocol::WORKSPACE_GIT_CAPABILITY.to_owned());
        }
        if profile.turn_follow_enabled() {
            capabilities
                .0
                .push(AGENT_CHAT_TURN_FOLLOW_CAPABILITY.to_owned());
        }
        if profile.agent_chat_permissions_enabled() {
            capabilities
                .0
                .push(gent_protocol::AGENT_CHAT_PERMISSIONS_CAPABILITY.to_owned());
        }
        if profile.conversation_activity_enabled() {
            capabilities
                .0
                .push(CONVERSATION_ACTIVITY_CAPABILITY.to_owned());
        }
    }
    if profile.reviewed_plans_enabled() {
        capabilities.0.push(REVIEWED_PLAN_CAPABILITY.to_owned());
    }
    if profile.provider_readiness_enabled() {
        capabilities
            .0
            .push(PROVIDER_READINESS_CAPABILITY.to_owned());
    }
    if profile.prompt_provider_provision_enabled() {
        capabilities
            .0
            .push(PROMPT_PROVIDER_PROVISION_CAPABILITY.to_owned());
    }
    if profile.runtime_update_check_enabled() {
        capabilities
            .0
            .push(RUNTIME_UPDATE_CHECK_CAPABILITY.to_owned());
    }
    if profile.runtime_maintenance_enabled() {
        capabilities
            .0
            .push(RUNTIME_MAINTENANCE_CAPABILITY.to_owned());
    }
    if profile.local_models_enabled() {
        capabilities.0.push(LOCAL_MODELS_CAPABILITY.to_owned());
    }
    capabilities
        .0
        .push(CONVERSATION_STATUS_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_INDEX_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_TIMELINE_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_CONTENT_CAPABILITY.to_owned());
    capabilities
}

/// Converts typed handler observations into their stable wire representation.
#[must_use]
pub fn capability_set(observed: impl IntoIterator<Item = RuntimeCapability>) -> CapabilitySet {
    CapabilitySet(
        observed
            .into_iter()
            .map(|capability| capability.wire_name().into())
            .collect(),
    )
}
#[path = "catalog_reconciliation.rs"]
mod reconciliation;
pub use reconciliation::validate_observed_capabilities;
#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
