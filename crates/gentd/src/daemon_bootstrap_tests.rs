use crate::authority_profile::{
    AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest, ValidatedAuthorityProfile,
};

use clap::Parser;

use super::{Args, ProviderLogin, enforce_hard_observer};

#[test]
fn provider_login_is_a_direct_one_off_command() {
    let args = Args::try_parse_from(["gentd", "--provider-login", "codex"]).unwrap();
    assert!(matches!(args.provider_login, Some(ProviderLogin::Codex)));
    assert!(!args.standalone_authority);
}

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
