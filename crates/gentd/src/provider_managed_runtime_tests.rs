use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use gent_ports::{LedgerError, ProvisionedProviderLockReader};
use gent_types::{
    ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock,
};

use super::*;
use crate::{
    ordinary_authority_release::fixture, standalone_authority_release::StandaloneAuthorityRelease,
};

#[derive(Clone)]
struct Installed(Arc<Mutex<ProvisionedProviderInstallation>>);
impl ProvisionedProviderLockReader for Installed {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        Ok(Some(self.0.lock().unwrap().clone()))
    }
}

struct Fixture {
    root: tempfile::TempDir,
    executables: ProviderExecutables,
    installed: Installed,
    launch: RunVersionLock,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = fixture::runtime(&root.path().join("runtime"));
        let path = crate::standalone_provider_setup::provider_executable(
            &crate::standalone_provider_setup::provider_prefix(root.path()),
            gent_protocol::DependencyProvider::Claude,
        )
        .unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "managed native image").unwrap();
        let launch = gent_drivers::lock::capture("claude", &path, "0.1.0", "claude-0.1.0").unwrap();
        let signer = SigningKey::from_bytes(&[91; 32]);
        let path = root.path().join("release.json");
        let release_document = fixture::release_for_provider(
            &signer,
            runtime.node_digest_sha256(),
            "claude",
            "0.1.0",
            &launch.digest_sha256,
        );
        std::fs::write(&path, serde_json::to_vec(&release_document).unwrap()).unwrap();
        let release = StandaloneAuthorityRelease::configured(
            path,
            &[format!(
                "root:{}",
                hex::encode(signer.verifying_key().as_bytes())
            )],
            runtime,
        )
        .unwrap();
        let verified = release.load(crate::startup::unix_seconds()).unwrap();
        let package = verified
            .package_policy()
            .approved_package("claude", crate::startup::unix_seconds())
            .unwrap();
        let provenance = ProviderInstallProvenance {
            package_name: package.package_name,
            package_version: package.version,
            package_integrity: package.integrity,
            package_policy_digest_sha256: package.package_policy_digest_sha256,
            node_runtime_digest_sha256: release.runtime().unwrap().node_digest_sha256().into(),
            release_artifact_digest_sha256: verified.artifact_digest_sha256().into(),
            receipt_fingerprint_sha256: "a".repeat(64),
        };
        let installed = Installed(Arc::new(Mutex::new(ProvisionedProviderInstallation {
            lock: ProvisionedProviderLock {
                run_lock: launch.clone(),
            },
            provenance,
        })));
        let executables = ProviderExecutables::standalone(
            None,
            None,
            root.path(),
            installed.clone(),
            Some(release),
        );
        Self {
            root,
            executables,
            installed,
            launch,
        }
    }
}

#[test]
#[cfg(windows)]
fn managed_resources_follow_only_the_receipt_bound_generation() {
    let fixture = Fixture::new();
    let prefix = crate::standalone_provider_setup::provider_generation_prefix(
        &crate::standalone_provider_setup::provider_prefix(fixture.root.path()),
        &"a".repeat(64),
    )
    .unwrap();
    let path = crate::standalone_provider_setup::provider_executable(
        &prefix,
        gent_protocol::DependencyProvider::Claude,
    )
    .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "managed native image").unwrap();
    let launch = gent_drivers::lock::capture("claude", &path, "0.1.0", "claude-0.1.0").unwrap();
    fixture.installed.0.lock().unwrap().lock.run_lock = launch.clone();
    let runtime = fixture
        .executables
        .managed_runtime(AgentChatProvider::Claude, &launch)
        .unwrap();
    assert!(
        runtime
            .package_root()
            .starts_with(prefix.canonicalize().unwrap())
    );
    fixture
        .installed
        .0
        .lock()
        .unwrap()
        .provenance
        .receipt_fingerprint_sha256 = "b".repeat(64);
    assert!(
        fixture
            .executables
            .managed_runtime(AgentChatProvider::Claude, &launch)
            .is_err()
    );
}

#[test]
fn refreshing_metadata_never_replaces_the_settled_content_digest() {
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.launch.canonical_path,
        "different installed content",
    )
    .unwrap();
    assert!(super::current_identity(&fixture.launch).is_none());
}

#[test]
fn signed_managed_image_package_and_node_produce_a_rechecked_runtime() {
    let fixture = Fixture::new();
    let mut launch = fixture.launch.clone();
    launch.version = super::super::LOCAL_VERSION.into();
    launch.compatibility_entry = super::super::LOCAL_ENTRY.into();
    let runtime = fixture
        .executables
        .managed_runtime(AgentChatProvider::Claude, &launch)
        .unwrap();
    assert_eq!(runtime.executable(), &launch);
    assert_eq!(runtime.package().provider, "claude");
    assert!(
        runtime
            .node()
            .node_path()
            .starts_with(fixture.root.path().canonicalize().unwrap())
    );
    runtime.recheck().unwrap();
}

#[test]
fn package_provenance_and_node_tampering_cannot_be_blessed_by_recapture() {
    let fixture = Fixture::new();
    fixture
        .installed
        .0
        .lock()
        .unwrap()
        .provenance
        .package_integrity = "sha512-substitute".into();
    assert!(
        fixture
            .executables
            .managed_runtime(AgentChatProvider::Claude, &fixture.launch)
            .is_err()
    );
    let fixture = Fixture::new();
    let runtime = fixture
        .executables
        .managed_runtime(AgentChatProvider::Claude, &fixture.launch)
        .unwrap();
    std::fs::write(runtime.node().npm_cli_path(), "substituted npm").unwrap();
    assert!(runtime.recheck().is_err());
    assert!(
        fixture
            .executables
            .managed_runtime(AgentChatProvider::Claude, &fixture.launch)
            .is_err()
    );
}

#[test]
fn explicit_or_wrong_image_paths_do_not_acquire_managed_resource_authority() {
    let fixture = Fixture::new();
    let explicit =
        ProviderExecutables::explicit(Some(fixture.launch.canonical_path.clone().into()), None);
    assert!(
        explicit
            .managed_runtime(AgentChatProvider::Claude, &fixture.launch)
            .is_err()
    );
    let mut other = fixture.launch.clone();
    other.canonical_path = fixture
        .root
        .path()
        .join("same-bytes.exe")
        .to_string_lossy()
        .into_owned();
    std::fs::write(&other.canonical_path, "managed native image").unwrap();
    assert!(
        fixture
            .executables
            .managed_runtime(AgentChatProvider::Claude, &other)
            .is_err()
    );
}
