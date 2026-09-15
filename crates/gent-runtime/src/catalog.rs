//! Capability reconciliation is pure: live declarations must match observed behavior.

use gent_protocol::{
    AGENT_CHAT_CHECKPOINT_CAPABILITY, AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY,
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_SESSIONS_CAPABILITY, AGENT_CHAT_SIDE_QUESTION_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY,
    AGENT_CHAT_TURN_FOLLOW_CAPABILITY, ATTACHMENTS_CAPABILITY, AUTOMATIONS_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_CONTENT_CAPABILITY,
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY, FORGE_CONNECTORS_CAPABILITY,
    GOAL_CAPABILITY, LOCAL_MODELS_CAPABILITY, ORCHESTRATION_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_AUTH_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    REVIEWED_PLAN_CAPABILITY, RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
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
    let agent_chat = profile.agent_chat_enabled();
    let profiled = [
        (agent_chat, RuntimeCapability::AgentChatIntents.wire_name()),
        (agent_chat, AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY),
        (agent_chat, AGENT_CHAT_CONVERSATIONS_CAPABILITY),
        (agent_chat, AGENT_CHAT_TRANSCRIPT_CAPABILITY),
        (agent_chat, AGENT_CHAT_SESSIONS_CAPABILITY),
        (agent_chat, GOAL_CAPABILITY),
        (agent_chat, ORCHESTRATION_CAPABILITY),
        (agent_chat, AUTOMATIONS_CAPABILITY),
        (agent_chat, FORGE_CONNECTORS_CAPABILITY),
        (agent_chat, AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY),
        (agent_chat, AGENT_CHAT_CHECKPOINT_CAPABILITY),
        (agent_chat, AGENT_CHAT_SIDE_QUESTION_CAPABILITY),
        (
            agent_chat,
            gent_protocol::agent_chat_commands::AGENT_CHAT_COMMANDS_CAPABILITY,
        ),
        (
            agent_chat && profile.prompt_templates_enabled(),
            gent_protocol::PROMPT_TEMPLATES_CAPABILITY,
        ),
        (
            agent_chat && profile.has(RuntimeCapabilityFeature::WorkspaceDocuments),
            gent_protocol::WORKSPACE_DOCUMENTS_CAPABILITY,
        ),
        (
            agent_chat && profile.has(RuntimeCapabilityFeature::WorkspaceGit),
            gent_protocol::WORKSPACE_GIT_CAPABILITY,
        ),
        (
            profile.turn_follow_enabled(),
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
        ),
        (
            profile.agent_chat_permissions_enabled(),
            gent_protocol::AGENT_CHAT_PERMISSIONS_CAPABILITY,
        ),
        (
            profile.conversation_activity_enabled(),
            CONVERSATION_ACTIVITY_CAPABILITY,
        ),
        (
            profile.agent_chat_projection_enabled(),
            gent_protocol::AGENT_CHAT_PROJECTION_CAPABILITY,
        ),
        (profile.reviewed_plans_enabled(), REVIEWED_PLAN_CAPABILITY),
        (
            profile.provider_readiness_enabled(),
            PROVIDER_READINESS_CAPABILITY,
        ),
        (profile.provider_auth_enabled(), PROVIDER_AUTH_CAPABILITY),
        (
            profile.prompt_provider_provision_enabled(),
            PROMPT_PROVIDER_PROVISION_CAPABILITY,
        ),
        (
            profile.runtime_update_check_enabled(),
            RUNTIME_UPDATE_CHECK_CAPABILITY,
        ),
        (
            profile.runtime_maintenance_enabled(),
            RUNTIME_MAINTENANCE_CAPABILITY,
        ),
        (profile.local_models_enabled(), LOCAL_MODELS_CAPABILITY),
        (
            profile.local_models_enabled(),
            gent_protocol::model_catalog::MODEL_CATALOG_CAPABILITY,
        ),
        (true, CONVERSATION_STATUS_CAPABILITY),
        (true, CONVERSATION_INDEX_CAPABILITY),
        (true, CONVERSATION_TIMELINE_CAPABILITY),
        (true, CONVERSATION_CONTENT_CAPABILITY),
    ];
    let mut capabilities = capability_set(DECLARED);
    capabilities.0.extend(
        profiled
            .into_iter()
            .filter(|(enabled, _)| *enabled)
            .map(|(_, capability)| capability.to_owned()),
    );
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
#[path = "catalog_profile.rs"]
mod profile;
pub use profile::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
#[path = "catalog_reconciliation.rs"]
mod reconciliation;
pub use reconciliation::validate_observed_capabilities;
#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
