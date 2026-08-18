//! Capability-catalog reconciliation is pure: declarations must match observed behavior.

use gent_ports::CapabilityCatalogLedger;
use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, ATTACHMENTS_CAPABILITY, CONVERSATION_INDEX_CAPABILITY,
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY,
    GOAL_CAPABILITY, ORCHESTRATION_CAPABILITY, REVIEWED_PLAN_CAPABILITY,
    RUNTIME_MAINTENANCE_CAPABILITY, RUNTIME_UPDATE_CHECK_CAPABILITY,
};
use gent_types::{CapabilityCatalogRecord, CapabilitySet};

#[cfg(unix)]
use gent_protocol::CONVERSATION_CONTENT_CAPABILITY;

use crate::Coordinator;
use crate::RuntimeError;

/// A capability the runtime may advertise after its transport proves a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapability {
    AgentChatIntents,
    Attachments,
    Decisions,
    EventResync,
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
            Self::EventResync => "event-resync",
            Self::EventStream => EVENT_STREAM_CAPABILITY,
            Self::Events => "events",
            Self::HostEpoch => "host-epoch",
            Self::PermissionPolicies => gent_protocol::PERMISSION_POLICY_CAPABILITY,
            Self::Receipts => "receipts",
        }
    }
}

const DECLARED: [RuntimeCapability; 8] = [
    RuntimeCapability::Attachments,
    RuntimeCapability::Decisions,
    RuntimeCapability::EventResync,
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
    declared_capabilities_with_profiles(agent_chat_enabled, false, false)
}

/// Returns the catalog for explicit authority profiles that have concrete handlers.
#[must_use]
pub fn declared_capabilities_with_profiles(
    agent_chat_enabled: bool,
    runtime_update_check_enabled: bool,
    runtime_maintenance_enabled: bool,
) -> CapabilitySet {
    let mut capabilities = capability_set(DECLARED);
    if agent_chat_enabled {
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
        capabilities.0.push(REVIEWED_PLAN_CAPABILITY.to_owned());
        capabilities.0.push(ORCHESTRATION_CAPABILITY.to_owned());
    }
    if runtime_update_check_enabled {
        capabilities
            .0
            .push(RUNTIME_UPDATE_CHECK_CAPABILITY.to_owned());
    }
    if runtime_maintenance_enabled {
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
    let declared = declared_capabilities_with_profiles(
        observed
            .0
            .iter()
            .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY),
        observed
            .0
            .iter()
            .any(|capability| capability == RUNTIME_UPDATE_CHECK_CAPABILITY),
        observed
            .0
            .iter()
            .any(|capability| capability == RUNTIME_MAINTENANCE_CAPABILITY),
    );
    reconcile(&declared, observed)?;
    Ok(declared)
}

impl<L> Coordinator<L>
where
    L: CapabilityCatalogLedger,
{
    /// Persists the complete capability set validated for this daemon process.
    ///
    /// # Errors
    /// Returns an error when durable storage rejects the catalog snapshot.
    pub fn persist_capability_catalog(&self) -> Result<(), RuntimeError> {
        self.ledger
            .save_capability_catalog(&CapabilityCatalogRecord {
                schema_version: 1,
                capabilities: self.capabilities.clone(),
            })?;
        Ok(())
    }
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
