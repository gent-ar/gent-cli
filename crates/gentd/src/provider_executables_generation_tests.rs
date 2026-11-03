use super::*;

fn generation_path(scenario: &Scenario, fingerprint: &str) -> std::path::PathBuf {
    let prefix = crate::standalone_provider_setup::provider_generation_prefix(
        &provider_prefix(scenario.directory.path()),
        fingerprint,
    )
    .unwrap();
    provider_executable(&prefix, DependencyProvider::Codex).unwrap()
}

#[test]
fn only_the_receipt_bound_generation_is_ready_and_visible_to_doctor() {
    let scenario = Scenario::new();
    let path = generation_path(&scenario, &"d".repeat(64));
    let lock = scenario.install_at(&path, "receipt-isolated native codex");
    scenario.sign(&lock.digest_sha256, false);
    let executables = scenario.executables(true);
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::Ready(lock.clone())
    );
    let report = crate::dependency_catalog::DependencyCatalog::default()
        .for_standalone(crate::dependency_catalog::standalone::StandaloneDoctor {
            executables: executables.clone(),
            provider_release: true,
            provisioned: Some(std::sync::Arc::new(scenario.installed.clone())),
            ..Default::default()
        })
        .doctor();
    let codex = report
        .public_providers
        .iter()
        .find(|provider| provider.provider == "codex")
        .unwrap();
    assert_eq!(
        codex.executable.as_ref().unwrap().canonical_path,
        lock.canonical_path
    );

    let other = generation_path(&scenario, &"e".repeat(64));
    let other_lock = scenario.install_at(&other, "other attempt");
    scenario.sign(&other_lock.digest_sha256, false);
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::InvalidInstallation
    );
    assert!(executables.executable(AgentChatProvider::Codex).is_none());
}

#[test]
fn unrecorded_generation_and_legacy_directories_are_not_discovered() {
    let scenario = Scenario::new();
    let path = generation_path(&scenario, &"d".repeat(64));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "unproven package").unwrap();
    fs::create_dir_all(scenario.native_path().parent().unwrap()).unwrap();
    fs::write(scenario.native_path(), "unproven legacy package").unwrap();
    let report = crate::dependency_catalog::DependencyCatalog::with_private_prefix(
        crate::CompatibilityAssessment::default(),
        provider_prefix(scenario.directory.path()),
    )
    .for_standalone(crate::dependency_catalog::standalone::StandaloneDoctor {
        executables: scenario.executables(true),
        ..Default::default()
    })
    .doctor();
    assert!(
        report
            .public_providers
            .iter()
            .all(|provider| provider.executable.is_none())
    );
}

#[test]
fn a_signed_legacy_verified_install_remains_supported() {
    let scenario = Scenario::new();
    let (lock, executables) = scenario.signed_install();
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::Ready(lock)
    );
}
