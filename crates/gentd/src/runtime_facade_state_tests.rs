use gent_runtime::catalog::{RuntimeCapabilityProfile, declared_capabilities_with_profiles};

use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
};

#[test]
fn preopened_composition_state_builds_the_identical_observer_facade() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::default();
    let capabilities = declared_capabilities_with_profiles(&profile);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert_eq!(state.data_dir(), directory.path());
    assert_eq!(
        state.coordinator().status().unwrap().capabilities,
        capabilities
    );
    assert_eq!(
        state.compatibility().manifest_sha256(),
        CompatibilityAssessment::default().manifest_sha256()
    );

    let runtime = RuntimeFacade::from_state(state, None).unwrap();
    assert_eq!(runtime.capabilities().unwrap(), capabilities);
    assert_eq!(runtime.status().unwrap().capabilities, capabilities);
}
