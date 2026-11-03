use std::{
    fs,
    time::{Duration, SystemTime},
};

use gent_drivers::lock::capture;
use gent_ports::PublicProviderResolver;
use gent_protocol::DependencyProvider;
use gent_types::AgentChatProvider;

use super::ProviderExecutables;
use crate::{
    local_provider_locks::LocalProviderLockError,
    private_provider_readiness::PrivateProviderReadiness,
    standalone_provider_setup::{provider_executable, provider_prefix},
};

#[path = "provider_executables_test_support.rs"]
mod support;
use support::{Scenario, VERSION, readiness};

#[test]
fn signed_native_binary_at_the_platform_layout_is_ready_and_launchable() {
    let scenario = Scenario::new();
    let (lock, executables) = scenario.signed_install();

    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::Ready(lock.clone())
    );
    assert_eq!(
        executables
            .executable(AgentChatProvider::Codex)
            .unwrap()
            .to_string_lossy(),
        lock.canonical_path
    );
    let launch = executables
        .locks(AgentChatProvider::Codex)
        .unwrap()
        .resolve("codex")
        .unwrap();
    assert_eq!(launch.digest_sha256, lock.digest_sha256);
    assert_eq!(launch.compatibility_entry, "standalone-local-v1");
}

#[test]
fn missing_unreadable_and_claurst_installations_never_launch() {
    let scenario = Scenario::new();
    let executables = scenario.executables(true);
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::InstallReview
    );
    assert_eq!(
        executables.locks(AgentChatProvider::Codex).unwrap_err(),
        LocalProviderLockError::PathUnavailable
    );
    *scenario.installed.0.lock().unwrap() = Err(());
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::Unavailable
    );
    assert_eq!(
        executables.installed_readiness(AgentChatProvider::Claurst),
        PrivateProviderReadiness::ClaurstUnavailable
    );
}

#[test]
fn unsigned_digest_shim_layout_or_missing_release_is_an_invalid_installation() {
    let scenario = Scenario::new();
    let native = scenario.install_at(&scenario.native_path(), "native codex");
    scenario.sign(&"e".repeat(64), false);
    let unsigned = scenario.executables(true);
    assert_eq!(
        readiness(&unsigned),
        PrivateProviderReadiness::InvalidInstallation
    );
    assert!(unsigned.executable(AgentChatProvider::Codex).is_none());
    assert_eq!(
        unsigned.locks(AgentChatProvider::Codex).unwrap_err(),
        LocalProviderLockError::InvalidInstallation
    );

    scenario.sign(&native.digest_sha256, false);
    assert_eq!(
        readiness(&scenario.executables(false)),
        PrivateProviderReadiness::InvalidInstallation
    );

    let shim = scenario.install_at(
        &provider_prefix(scenario.directory.path()).join("bin/codex"),
        "native codex",
    );
    scenario.sign(&shim.digest_sha256, false);
    assert_eq!(
        readiness(&scenario.executables(true)),
        PrivateProviderReadiness::InvalidInstallation
    );
}

#[test]
fn a_tampered_binary_is_refused_at_the_next_launch() {
    let scenario = Scenario::new();
    let (_, executables) = scenario.signed_install();
    let locks = executables.locks(AgentChatProvider::Codex).unwrap();
    assert!(locks.resolve("codex").is_ok());

    fs::write(scenario.native_path(), "native codeX").unwrap();
    assert_eq!(
        readiness(&executables),
        PrivateProviderReadiness::InvalidInstallation
    );
    assert!(
        locks.resolve("codex").is_err(),
        "same size, new content and mtime"
    );

    fs::write(scenario.native_path(), "a longer tampered codex").unwrap();
    assert!(locks.resolve("codex").is_err(), "changed size");
}

#[test]
fn a_touched_binary_with_identical_bytes_is_rehashed_and_stays_ready() {
    let scenario = Scenario::new();
    let (lock, executables) = scenario.signed_install();
    let native = scenario.native_path();
    let later = SystemTime::now() + Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(&native)
        .unwrap()
        .set_modified(later)
        .unwrap();

    let PrivateProviderReadiness::Ready(current) = readiness(&executables) else {
        panic!("identical bytes with a new mtime must be rehashed and accepted");
    };
    assert_eq!(current.digest_sha256, lock.digest_sha256);
    assert_ne!(current.file_identity, lock.file_identity);
    assert!(gent_drivers::lock::recheck(&current).is_ok());
}

#[test]
fn every_launch_reloads_the_signed_release() {
    let scenario = Scenario::new();
    let (lock, executables) = scenario.signed_install();
    let locks = executables.locks(AgentChatProvider::Codex).unwrap();

    scenario.sign(&lock.digest_sha256, true);
    assert!(
        locks.resolve("codex").is_err(),
        "a revoked release must stop the next launch"
    );
    scenario.sign(&"f".repeat(64), false);
    assert!(
        locks.resolve("codex").is_err(),
        "a reloaded release that no longer authorizes the digest"
    );
    scenario.sign(&lock.digest_sha256, false);
    assert!(locks.resolve("codex").is_ok());
}

#[test]
fn a_reinstalled_binary_is_launched_by_the_existing_resolver() {
    let scenario = Scenario::new();
    let (_, executables) = scenario.signed_install();
    let locks = executables.locks(AgentChatProvider::Codex).unwrap();
    fs::remove_file(scenario.native_path()).unwrap();
    assert!(locks.resolve("codex").is_err());

    let reinstalled = scenario.install_at(&scenario.native_path(), "reinstalled codex");
    scenario.sign(&reinstalled.digest_sha256, false);
    assert_eq!(
        locks.resolve("codex").unwrap().digest_sha256,
        reinstalled.digest_sha256
    );
}

#[test]
fn an_explicit_developer_executable_is_used_without_installed_verification() {
    let scenario = Scenario::new();
    let explicit = scenario.directory.path().join("dev-codex");
    fs::write(&explicit, "developer codex").unwrap();
    let executables = ProviderExecutables::standalone(
        None,
        Some(explicit.clone()),
        scenario.directory.path(),
        scenario.installed.clone(),
        None,
    );
    assert_eq!(
        executables.executable(AgentChatProvider::Codex),
        Some(explicit)
    );
    assert!(
        executables
            .locks(AgentChatProvider::Codex)
            .unwrap()
            .resolve("codex")
            .is_ok()
    );
    assert!(executables.executable(AgentChatProvider::Claude).is_none());
}

#[test]
fn an_installed_claude_needs_a_release_that_authorizes_claude() {
    let scenario = Scenario::new();
    let claude = provider_executable(
        &provider_prefix(scenario.directory.path()),
        DependencyProvider::Claude,
    )
    .unwrap();
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(&claude, "native claude").unwrap();
    let lock = capture("claude", &claude, VERSION, "codex-0.1.0").unwrap();
    *scenario.installed.0.lock().unwrap() = Ok(Some(lock.clone()));
    scenario.sign(&lock.digest_sha256, false);
    let executables = scenario.executables(true);
    assert_eq!(
        executables.installed_readiness(AgentChatProvider::Claude),
        PrivateProviderReadiness::InvalidInstallation
    );
    assert_eq!(
        executables.locks(AgentChatProvider::Claude).unwrap_err(),
        LocalProviderLockError::InvalidInstallation
    );
}

#[test]
fn the_revision_follows_the_verified_installation_and_explicit_rebuilds() {
    let scenario = Scenario::new();
    let executables = scenario.executables(true);
    assert_eq!(executables.revision(AgentChatProvider::Codex), None);
    let (installed, executables) = scenario.signed_install();
    let first = executables.revision(AgentChatProvider::Codex).unwrap();
    assert!(first.contains(&installed.digest_sha256));
    assert_eq!(
        executables.revision(AgentChatProvider::Codex),
        Some(first.clone())
    );

    let upgraded = scenario.install_at(&scenario.native_path(), "upgraded codex");
    scenario.sign(&upgraded.digest_sha256, false);
    let second = executables.revision(AgentChatProvider::Codex).unwrap();
    assert_ne!(second, first);
    fs::write(scenario.native_path(), "tampered codex").unwrap();
    assert_eq!(executables.revision(AgentChatProvider::Codex), None);

    let explicit = scenario.directory.path().join("dev-claude");
    fs::write(&explicit, "developer claude").unwrap();
    let developer = ProviderExecutables::explicit(Some(explicit.clone()), None);
    let built = developer.revision(AgentChatProvider::Claude).unwrap();
    fs::write(&explicit, "rebuilt developer claude").unwrap();
    assert_ne!(
        developer.revision(AgentChatProvider::Claude).unwrap(),
        built
    );
}

#[path = "provider_executables_generation_tests.rs"]
mod generations;
