use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
};

use fs2::FileExt;
use gent_ports::Ledger;

use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    bootstrap: PathBuf,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new(version: &str) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        for name in required_files() {
            let path = bootstrap.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name.as_bytes()).unwrap();
        }
        fs::write(
            bootstrap.join("bootstrap.json"),
            serde_json::json!({"version":version,"target":expected_target().unwrap()}).to_string(),
        )
        .unwrap();
        Self {
            bootstrap,
            root: temporary.path().join("runtime"),
            data: temporary.path().join("data"),
            _temporary: temporary,
        }
    }
    fn activate(&self) -> Result<ActivationResult, String> {
        activate_at_expected(
            &self.bootstrap,
            &self.root,
            &self.data,
            |_, _| Ok(()),
            None,
            None,
            None,
        )
    }
    fn ledger(&self) -> gent_store::SqliteLedger {
        fs::create_dir_all(&self.data).unwrap();
        gent_store::SqliteLedger::open(self.data.join("gent.db")).unwrap()
    }
}

#[test]
fn activation_machine_probe_is_read_only_and_does_not_advertise_ipc_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let data = temporary.path().join("not-created");
    let value: serde_json::Value = serde_json::from_str(
        &execute(RuntimeCommand::ActivationCapabilities, data.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(value["schema"], 1);
    assert!(
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "runtime-activation-lease-v1")
    );
    assert!(
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "runtime-immutable-release-v1")
    );
    assert!(!data.exists());
}

#[test]
fn a_live_data_owner_blocks_default_and_explicit_activation_before_runtime_writes() {
    let fixture = Fixture::new("v0.2.4");
    fs::create_dir_all(&fixture.data).unwrap();
    let owner = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.data.join("gentd.lock"))
        .unwrap();
    owner.try_lock_exclusive().unwrap();
    assert!(fixture.activate().is_err());
    assert!(
        activate(
            fixture.bootstrap.clone(),
            Some(fixture.root.clone()),
            fixture.data.clone()
        )
        .is_err()
    );
    assert!(!fixture.root.exists());
}

#[test]
fn concurrent_store_activator_blocks_publication_even_for_another_data_scope() {
    let fixture = Fixture::new("v0.2.4");
    fs::create_dir_all(&fixture.root).unwrap();
    let store = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.root.join(".install.lock"))
        .unwrap();
    store.try_lock_exclusive().unwrap();
    assert!(fixture.activate().is_err());
    assert!(!fixture.root.join("releases").exists());
}

#[test]
fn same_version_repair_preserves_other_owners_old_runtime_files() {
    let fixture = Fixture::new("v0.2.4");
    let first = fixture.activate().unwrap();
    let old = first.daemon_path.parent().unwrap();
    let node = old.join(if cfg!(windows) {
        "runtime/node/bin/node.exe"
    } else {
        "runtime/node/bin/node"
    });
    let original = fs::read(&node).unwrap();
    let _other_owner = fs::File::open(&node).unwrap();
    fs::write(
        fixture.bootstrap.join(if cfg!(windows) {
            "runtime/node/bin/node.exe"
        } else {
            "runtime/node/bin/node"
        }),
        "new node fixture",
    )
    .unwrap();
    let repaired = fixture.activate().unwrap();
    assert_ne!(first.selected_release, repaired.selected_release);
    assert_eq!(fs::read(node).unwrap(), original);
    assert!(first.daemon_path.exists());
    assert!(repaired.daemon_path.is_file());
    let repeated = fixture.activate().unwrap();
    assert_eq!(repeated.selected_release, repaired.selected_release);
}

#[test]
fn planned_digest_and_epoch_are_checked_before_publication() {
    let fixture = Fixture::new("v0.2.4");
    let ledger = fixture.ledger();
    ledger.close_ingress(gent_types::HostEpoch(1)).unwrap();
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            Some(&"0".repeat(64)),
            Some(1),
            None,
        )
        .is_err()
    );
    assert!(!fixture.root.join("releases").exists());
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            None,
            Some(2),
            None,
        )
        .is_err()
    );
    assert_eq!(ledger.host_ingress().unwrap().epoch.0, 1);
}

#[test]
fn acknowledged_epoch_requires_closed_ingress_and_returns_exact_successor_plan() {
    let fixture = Fixture::new("v0.2.4");
    let ledger = fixture.ledger();
    let daemon = fixture
        .bootstrap
        .join(if cfg!(windows) { "gentd.exe" } else { "gentd" });
    let planned = stage::file_digest(&daemon).unwrap();
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            Some(&planned),
            Some(1),
            None,
        )
        .is_err()
    );
    ledger.close_ingress(gent_types::HostEpoch(1)).unwrap();
    let result = activate_at_expected(
        &fixture.bootstrap,
        &fixture.root,
        &fixture.data,
        |_, _| Ok(()),
        Some(&planned),
        Some(1),
        None,
    )
    .unwrap();
    assert_eq!(result.origin_host_epoch, Some(1));
    assert_eq!(result.executable_digest_sha256, planned);
    assert_eq!(result.data_dir, fixture.data.canonicalize().unwrap());
    assert_eq!(
        ledger.host_ingress().unwrap().mode,
        gent_ports::IngressMode::Closed
    );
    assert_eq!(
        result.daemon_path,
        fixture
            .root
            .canonicalize()
            .unwrap()
            .join("releases")
            .join(&result.selected_release)
            .join(if cfg!(windows) { "gentd.exe" } else { "gentd" })
    );
}

#[test]
fn prepared_tree_identity_rejects_changed_non_daemon_files_before_publication() {
    let fixture = Fixture::new("v0.2.4");
    let identity = bootstrap_identity(&fixture.bootstrap).unwrap();
    assert!(!fixture.root.exists());
    fs::write(
        fixture.bootstrap.join("runtime-extra.txt"),
        "changed dependency",
    )
    .unwrap();
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            Some(&identity.executable_digest_sha256),
            None,
            Some(&identity.bootstrap_tree_sha256),
        )
        .is_err()
    );
    assert!(!fixture.root.join("releases").exists());
    assert!(!fixture.root.join("current.json").exists());
}

#[path = "runtime_activation_cold_start_tests.rs"]
mod cold_start_tests;
