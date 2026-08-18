use gent_ports::CapabilityCatalogLedger;
use gent_runtime::catalog::declared_capabilities;

use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
};

#[test]
fn preopened_composition_state_builds_the_identical_observer_facade() {
    let directory = tempfile::tempdir().unwrap();
    let capabilities = declared_capabilities();
    let state = DaemonCompositionState::open(
        directory.path(),
        &capabilities,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert_eq!(state.data_dir(), directory.path());
    assert_eq!(
        state.coordinator().status().unwrap().capabilities,
        capabilities
    );
    assert_eq!(
        state
            .ledger()
            .capability_catalog()
            .unwrap()
            .unwrap()
            .capabilities,
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
