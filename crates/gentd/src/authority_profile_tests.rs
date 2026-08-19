use super::{
    AuthorityProfileConfig, AuthorityProfileError, DeferredSurfaceRequest, McpApproval, McpRequest,
    PublicDriverApproval, PublicDriverRequest, ValidatedAuthorityProfile,
};
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, PROVIDER_AUTH_CAPABILITY,
};

fn approval() -> PublicDriverApproval {
    PublicDriverApproval {
        evidence_reference: "public-driver-evidence-2026-08-16".into(),
        compatibility_manifest_sha256: "a".repeat(64),
    }
}

#[test]
fn default_profile_is_observer_and_all_deferred_surfaces_are_disabled() {
    let profile = AuthorityProfileConfig::default().validate().unwrap();
    assert_eq!(profile, ValidatedAuthorityProfile::Observer);
    assert!(matches!(profile, ValidatedAuthorityProfile::Observer));
}

#[test]
fn observer_and_durable_chat_profiles_hide_every_dormant_provider_lifecycle_surface() {
    let observer = crate::transport::observed_capabilities(false, false, false, false);
    let durable_chat = crate::transport::observed_capabilities(true, false, false, false);
    assert!(crate::authority_profile::shipped_observer_profile().is_hard_observer());

    for capabilities in [&observer, &durable_chat] {
        for capability in [
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
            CONVERSATION_ACTIVITY_CAPABILITY,
            PROVIDER_AUTH_CAPABILITY,
        ] {
            assert!(
                !capabilities
                    .0
                    .iter()
                    .any(|advertised| advertised == capability),
                "dormant provider surface {capability} must not be negotiated"
            );
        }
    }
    assert!(
        !observer
            .0
            .iter()
            .any(|advertised| advertised == AGENT_CHAT_INTENTS_CAPABILITY)
    );
    assert!(
        durable_chat
            .0
            .iter()
            .any(|advertised| advertised == AGENT_CHAT_INTENTS_CAPABILITY),
        "durable chat persistence is distinct from provider lifecycle authority"
    );
}

#[test]
fn public_drivers_need_explicit_complete_approval_but_do_not_compose_a_runner() {
    let incomplete = AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: " ".into(),
            compatibility_manifest_sha256: "a".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    };
    assert_eq!(
        incomplete.validate(),
        Err(AuthorityProfileError::MissingEvidenceReference)
    );
    let profile = AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(approval()),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap();
    assert!(matches!(
        profile,
        ValidatedAuthorityProfile::PreparedPublicDrivers(_)
    ));
}

#[test]
fn unsupported_surfaces_are_rejected_before_any_future_provider_composition() {
    let profile = AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(approval()),
        git: DeferredSurfaceRequest::Requested,
        ..AuthorityProfileConfig::default()
    };
    assert_eq!(
        profile.validate(),
        Err(AuthorityProfileError::DeferredSurfaceRequested { surface: "Git" })
    );
}

#[test]
fn mcp_requires_evidence_and_a_pinned_registry_digest() {
    let incomplete = AuthorityProfileConfig {
        mcp: McpRequest::Approved(McpApproval {
            evidence_reference: " ".into(),
            registry_sha256: "a".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    };
    assert_eq!(
        incomplete.validate(),
        Err(AuthorityProfileError::MissingMcpEvidenceReference)
    );
    let profile = AuthorityProfileConfig {
        mcp: McpRequest::Approved(McpApproval {
            evidence_reference: "mcp-evidence".into(),
            registry_sha256: "a".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap();
    assert!(matches!(profile, ValidatedAuthorityProfile::PreparedMcp(_)));
}
