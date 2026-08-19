//! Capability reconciliation is pure: live declarations must match observed behavior.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY, ATTACHMENTS_CAPABILITY,
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY, GOAL_CAPABILITY,
    ORCHESTRATION_CAPABILITY, RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
};
use gent_types::CapabilitySet;

#[cfg(unix)]
use gent_protocol::CONVERSATION_CONTENT_CAPABILITY;

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
    TurnFollow,
    RuntimeUpdateCheck,
    RuntimeMaintenance,
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
    if profile.has(RuntimeCapabilityFeature::AgentChat) {
        capabilities
            .0
            .push(RuntimeCapability::AgentChatIntents.wire_name().into());
        capabilities
            .0
            .push(AGENT_CHAT_CONVERSATIONS_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_TRANSCRIPT_CAPABILITY.to_owned());
        capabilities.0.push(GOAL_CAPABILITY.to_owned());
        capabilities.0.push(ORCHESTRATION_CAPABILITY.to_owned());
        if profile.has(RuntimeCapabilityFeature::TurnFollow) {
            capabilities
                .0
                .push(AGENT_CHAT_TURN_FOLLOW_CAPABILITY.to_owned());
        }
    }
    if profile.has(RuntimeCapabilityFeature::RuntimeUpdateCheck) {
        capabilities
            .0
            .push(RUNTIME_UPDATE_CHECK_CAPABILITY.to_owned());
    }
    if profile.has(RuntimeCapabilityFeature::RuntimeMaintenance) {
        capabilities
            .0
            .push(RUNTIME_MAINTENANCE_CAPABILITY.to_owned());
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
    #[cfg(unix)]
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

/// Rejects a transport whose observed handlers drift from the runtime catalog.
///
/// # Errors
/// Returns the mismatch before any status or handshake can advertise capabilities.
pub fn validate_observed_capabilities(
    observed: &CapabilitySet,
) -> Result<CapabilitySet, CatalogError> {
    let profile = RuntimeCapabilityProfile::new(
        [
            AGENT_CHAT_INTENTS_CAPABILITY,
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
            RUNTIME_UPDATE_CHECK_CAPABILITY,
            RUNTIME_MAINTENANCE_CAPABILITY,
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
            RUNTIME_UPDATE_CHECK_CAPABILITY => RuntimeCapabilityFeature::RuntimeUpdateCheck,
            RUNTIME_MAINTENANCE_CAPABILITY => RuntimeCapabilityFeature::RuntimeMaintenance,
            _ => unreachable!("only known capability names are passed"),
        })
}

#[cfg(test)]
mod tests {
    use super::{CatalogError, declared_capabilities, reconcile, validate_observed_capabilities};
    use gent_types::CapabilitySet;

    #[test]
    fn declared_capability_drift_fails_the_build_gate() {
        assert_eq!(
            reconcile(
                &CapabilitySet(vec!["events".into()]),
                &CapabilitySet::default()
            ),
            Err(CatalogError::DeclaredButUnavailable("events".into()))
        );
    }

    #[test]
    fn exact_catalog_is_accepted() {
        let catalog = CapabilitySet(vec!["events".into(), "receipts".into()]);
        assert!(reconcile(&catalog, &catalog).is_ok());
    }

    #[test]
    fn typed_observations_cannot_add_an_undeclared_wire_capability() {
        let mut observed = declared_capabilities();
        observed.0.push("future-capability".into());
        assert_eq!(
            validate_observed_capabilities(&observed),
            Err(CatalogError::UndeclaredObserved("future-capability".into()))
        );
    }
}
