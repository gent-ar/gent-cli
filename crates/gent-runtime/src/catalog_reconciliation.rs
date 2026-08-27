use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, LOCAL_MODELS_CAPABILITY,
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_READINESS_CAPABILITY, REVIEWED_PLAN_CAPABILITY,
    RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
};
use gent_types::CapabilitySet;

use super::{
    CatalogError, RuntimeCapabilityFeature, RuntimeCapabilityProfile,
    declared_capabilities_with_profiles, reconcile,
};

pub fn validate_observed_capabilities(
    observed: &CapabilitySet,
) -> Result<CapabilitySet, CatalogError> {
    let profile = RuntimeCapabilityProfile::new(
        [
            AGENT_CHAT_INTENTS_CAPABILITY,
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
            CONVERSATION_ACTIVITY_CAPABILITY,
            REVIEWED_PLAN_CAPABILITY,
            PROVIDER_READINESS_CAPABILITY,
            PROMPT_PROVIDER_PROVISION_CAPABILITY,
            RUNTIME_UPDATE_CHECK_CAPABILITY,
            RUNTIME_MAINTENANCE_CAPABILITY,
            LOCAL_MODELS_CAPABILITY,
        ]
        .into_iter()
        .filter_map(|capability| feature_from_observed(capability, observed)),
    );
    let declared = declared_capabilities_with_profiles(&profile);
    reconcile(&declared, observed)?;
    Ok(declared)
}

fn feature_from_observed(
    capability: &str,
    observed: &CapabilitySet,
) -> Option<RuntimeCapabilityFeature> {
    observed
        .0
        .iter()
        .any(|actual| actual == capability)
        .then(|| match capability {
            AGENT_CHAT_INTENTS_CAPABILITY => RuntimeCapabilityFeature::AgentChat,
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY => RuntimeCapabilityFeature::TurnFollow,
            CONVERSATION_ACTIVITY_CAPABILITY => RuntimeCapabilityFeature::ConversationActivity,
            REVIEWED_PLAN_CAPABILITY => RuntimeCapabilityFeature::ReviewedPlans,
            PROVIDER_READINESS_CAPABILITY => RuntimeCapabilityFeature::ProviderReadiness,
            PROMPT_PROVIDER_PROVISION_CAPABILITY => {
                RuntimeCapabilityFeature::PromptProviderProvision
            }
            RUNTIME_UPDATE_CHECK_CAPABILITY => RuntimeCapabilityFeature::RuntimeUpdateCheck,
            RUNTIME_MAINTENANCE_CAPABILITY => RuntimeCapabilityFeature::RuntimeMaintenance,
            LOCAL_MODELS_CAPABILITY => RuntimeCapabilityFeature::LocalModels,
            _ => unreachable!("only known capability names are passed"),
        })
}
