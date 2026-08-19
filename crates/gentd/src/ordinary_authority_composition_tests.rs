use gent_runtime::catalog::RuntimeCapabilityProfile;

use super::owner;
use crate::{CompatibilityAssessment, runtime_facade::DaemonCompositionState};

#[test]
fn authority_owner_is_derived_from_the_active_daemon_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert_eq!(owner(&state).unwrap().0, "gentd-1");
}
