use crate::claurst_standalone_owner::ClaurstStandaloneLauncher;
use gent_types::PermissionMode;

#[test]
fn an_unsandboxed_run_has_no_binding_and_launches_on_the_host_in_every_mode() {
    let launcher = super::RoutedClaurstStandaloneLauncher::host();
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        assert!(launcher.validate_permission_mode(mode).is_ok());
    }
}
