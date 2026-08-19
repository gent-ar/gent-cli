use super::{
    CatalogError, RuntimeCapabilityFeature, RuntimeCapabilityProfile, declared_capabilities,
    declared_capabilities_with_profiles, reconcile, validate_observed_capabilities,
};
use gent_protocol::{
    AGENT_CHAT_TURN_FOLLOW_CAPABILITY, PROVIDER_READINESS_CAPABILITY, REVIEWED_PLAN_CAPABILITY,
};
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

#[test]
fn turn_follow_requires_agent_chat_in_the_single_typed_profile() {
    let profile = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::TurnFollow]);
    assert!(
        !declared_capabilities_with_profiles(&profile)
            .0
            .contains(&AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into())
    );
}

#[test]
fn reviewed_plans_are_advertised_only_by_their_typed_profile_feature() {
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ReviewedPlans,
    ]);
    assert!(
        declared_capabilities_with_profiles(&profile)
            .0
            .contains(&REVIEWED_PLAN_CAPABILITY.into())
    );
}

#[test]
fn provider_readiness_requires_its_typed_chat_profile_feature() {
    let absent = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]);
    assert!(
        !declared_capabilities_with_profiles(&absent)
            .0
            .contains(&PROVIDER_READINESS_CAPABILITY.into())
    );
    let enabled = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderReadiness,
    ]);
    assert!(
        declared_capabilities_with_profiles(&enabled)
            .0
            .contains(&PROVIDER_READINESS_CAPABILITY.into())
    );
}
