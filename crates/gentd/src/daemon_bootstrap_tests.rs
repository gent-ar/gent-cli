use crate::authority_profile::{
    AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest, ValidatedAuthorityProfile,
};

use super::enforce_hard_observer;

#[test]
fn shipped_profile_passes_the_bootstrap_observer_fence() {
    let profile = crate::authority_profile::shipped_observer_profile();
    assert!(profile.is_hard_observer());
    assert_eq!(enforce_hard_observer(&profile), Ok(()));
}

#[test]
fn bootstrap_fence_rejects_a_prepared_provider_profile() {
    let profile = AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: "reviewed-private-record".into(),
            compatibility_manifest_sha256: "a".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap();
    assert!(matches!(
        profile,
        ValidatedAuthorityProfile::PreparedPublicDrivers(_)
    ));
    assert!(!profile.is_hard_observer());
    assert_eq!(
        enforce_hard_observer(&profile),
        Err("gentd bootstrap only supports the hard observer profile".into())
    );
}
