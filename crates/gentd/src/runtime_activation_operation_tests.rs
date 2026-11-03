use super::*;
use gent_ports::Ledger;
use gent_protocol::runtime_activation::ActivationInspectionState as Observed;
use gent_protocol::runtime_activation::{
    ActivationCommand, ActivationLookup, ActivationOperationExpectation,
    ActivationOperationState as State, ExpectedActivationOwner,
};
use gent_types::HostEpoch;
use std::{fs, path::PathBuf};

struct Fixture {
    temp_root: tempfile::TempDir,
    command: ActivationCommand,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let bootstrap = base.join("bootstrap");
        for name in super::super::required_files() {
            let path = bootstrap.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name.as_bytes()).unwrap();
        }
        fs::write(
            bootstrap.join("bootstrap.json"),
            serde_json::json!({"version":"v0.2.4",
            "target":super::super::expected_target().unwrap()})
            .to_string(),
        )
        .unwrap();
        let data = base.join("data");
        fs::create_dir(&data).unwrap();
        let ledger = gent_store::SqliteLedger::open(data.join("gent.db")).unwrap();
        ledger.close_ingress(HostEpoch(1)).unwrap();
        drop(ledger);
        let identity = super::super::bootstrap_identity(&bootstrap).unwrap();
        let command = ActivationCommand {
            bootstrap_directory: bootstrap,
            runtime_root: base.join("runtime"),
            data_directory: data,
            expected_daemon_sha256: Some(identity.executable_digest_sha256),
            expected_bootstrap_tree_sha256: Some(identity.bootstrap_tree_sha256),
            expected_host_epoch: Some(1),
            enable_automatic_updates: false,
            expected_command_sha256: None,
            maintenance_daemon_sha256: "b".repeat(64),
            operation: Some(ActivationOperationExpectation {
                operation_id: uuid::Uuid::new_v4().to_string(),
                expected_owner: ExpectedActivationOwner {
                    process_id: 123,
                    executable_digest_sha256: "a".repeat(64),
                },
            }),
        };
        fs::create_dir(&command.runtime_root).unwrap();
        let command = plan::prepare(command).unwrap().command;
        Self {
            temp_root: temp,
            command,
        }
    }
    fn lookup(&self) -> ActivationLookup {
        let id = &self.command.operation.as_ref().unwrap().operation_id;
        let root = self.command.runtime_root.canonicalize().unwrap();
        let record = journal::read(&root, id).unwrap();
        ActivationLookup {
            operation_id: id.clone(),
            command_sha256: record.command_sha256,
            runtime_root: root,
            data_directory: self.command.data_directory.canonicalize().unwrap(),
        }
    }
    fn crash(&self, at: &str) {
        let result = std::panic::catch_unwind(|| {
            workflow::activate(self.command.clone(), |name| {
                assert!(name != at, "synthetic process crash at {name}");
                Ok(())
            })
            .unwrap();
        });
        assert!(result.is_err());
    }
}
#[test]
fn completed_operation_replay_does_not_create_another_generation() {
    let fixture = Fixture::new();
    let first = workflow::activate(fixture.command.clone(), |_| Ok(())).unwrap();
    let repeated =
        workflow::activate(fixture.command.clone(), |_| panic!("replay attempted")).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(first.state, State::Completed);
    assert_eq!(
        fs::read_dir(fixture.command.runtime_root.join("releases"))
            .unwrap()
            .count(),
        1
    );
    assert!(first.selected_release.contains(&first.command_sha256));
}
#[test]
fn published_crash_requires_explicit_adoption_and_never_forges_completion() {
    let fixture = Fixture::new();
    fixture.crash("pointerPublished");
    let lookup = fixture.lookup();
    let before = fs::read(
        journal::directory(&lookup.runtime_root, false)
            .unwrap()
            .join(format!("{}.json", lookup.operation_id)),
    )
    .unwrap();
    let status = status::inspect(&lookup).unwrap();
    assert_eq!(status.inspection.state, Observed::PublishedRecoveryRequired);
    assert_eq!(status.operation.state, State::PublicationIntent);
    assert!(status.operation.result.is_none());
    assert_eq!(
        before,
        fs::read(
            journal::directory(&lookup.runtime_root, false)
                .unwrap()
                .join(format!("{}.json", lookup.operation_id))
        )
        .unwrap()
    );
    let adopted = status::adopt(&lookup).unwrap();
    assert_eq!(adopted.operation.state, State::RecoveredPublication);
    assert_eq!(status::adopt(&lookup).unwrap(), adopted);
}
#[test]
fn prepublication_crashes_do_not_replay_or_allow_adoption() {
    for checkpoint in [
        "accepted",
        "preparing",
        "generationPublished",
        "publicationIntent",
    ] {
        let fixture = Fixture::new();
        fixture.crash(checkpoint);
        let lookup = fixture.lookup();
        assert!(status::adopt(&lookup).is_err());
        let old = fs::read_dir(&lookup.runtime_root).unwrap().count();
        workflow::activate(fixture.command.clone(), |_| panic!("replayed mutation")).unwrap();
        assert_eq!(fs::read_dir(&lookup.runtime_root).unwrap().count(), old);
    }
}
#[test]
fn completed_before_reply_survives_as_actual_completed_result() {
    let fixture = Fixture::new();
    fixture.crash("completedPersisted");
    assert_eq!(
        status::inspect(&fixture.lookup()).unwrap().operation.state,
        State::Completed
    );
}
#[test]
fn changed_command_or_lookup_identity_cannot_adopt_an_operation() {
    let fixture = Fixture::new();
    fixture.crash("pointerPublished");
    let mut changed = fixture.command.clone();
    changed
        .operation
        .as_mut()
        .unwrap()
        .expected_owner
        .process_id += 1;
    assert!(workflow::activate(changed, |_| Ok(())).is_err());
    let lookup = fixture.lookup();
    let mut wrong = lookup.clone();
    wrong.command_sha256 = "0".repeat(64);
    assert!(status::adopt(&wrong).is_err());
    wrong = lookup;
    wrong.data_directory = PathBuf::from("nonexistent-operation-data");
    assert!(status::inspect(&wrong).is_err());
    assert!(!wrong.data_directory.exists());
}
#[test]
fn lease_busy_status_is_read_only_and_fences_adoption() {
    use fs2::FileExt;
    let fixture = Fixture::new();
    fixture.crash("pointerPublished");
    let lookup = fixture.lookup();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lookup.runtime_root.join(".install.lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    assert_eq!(
        status::inspect(&lookup).unwrap().inspection.state,
        Observed::Busy
    );
    assert!(status::adopt(&lookup).is_err());
}
#[test]
fn changed_selected_bytes_and_closed_epoch_block_recovery() {
    let fixture = Fixture::new();
    fixture.crash("pointerPublished");
    let lookup = fixture.lookup();
    let selected = super::super::selected_release(&lookup.runtime_root)
        .unwrap()
        .unwrap();
    let daemon = lookup
        .runtime_root
        .join("releases")
        .join(selected)
        .join(if cfg!(windows) { "gentd.exe" } else { "gentd" });
    fs::write(daemon, "tampered").unwrap();
    assert!(status::adopt(&lookup).is_err());
    let ledger = gent_store::SqliteLedger::open(lookup.data_directory.join("gent.db")).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    drop(ledger);
    assert!(status::adopt(&lookup).is_err());
}
#[test]
fn absent_status_never_creates_root_data_or_operation_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing");
    let lookup = ActivationLookup {
        operation_id: uuid::Uuid::new_v4().to_string(),
        command_sha256: "1".repeat(64),
        runtime_root: root.clone(),
        data_directory: root.join("data"),
    };
    assert!(status::inspect(&lookup).is_err());
    assert!(!root.exists());
}
#[test]
fn generation_readers_keep_legacy_eight_and_accept_exact_full_operation_digest() {
    let target = super::super::expected_target().unwrap();
    for size in [8, 64] {
        assert!(super::super::is_generation(
            &format!("v0.2.4+{}.{}-{target}", "a".repeat(16), "b".repeat(size)),
            &target
        ));
    }
    for size in [0, 7, 9, 63, 65] {
        assert!(!super::super::is_generation(
            &format!("v0.2.4+{}.{}-{target}", "a".repeat(16), "b".repeat(size)),
            &target
        ));
    }
}
#[test]
fn read_only_plan_is_stable_and_does_not_accept_or_create_locks() {
    let fixture = Fixture::new();
    let root = &fixture.command.runtime_root;
    let before: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let a = plan::prepare(fixture.command.clone()).unwrap();
    let b = plan::prepare(fixture.command.clone()).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        before,
        fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>()
    );
    assert!(!root.join("activation-operations").exists());
    assert!(!root.join(".install.lock").exists());
}
#[test]
fn fresh_operation_does_not_borrow_another_operations_matching_generation() {
    let fixture = Fixture::new();
    let first = workflow::activate(fixture.command.clone(), |_| Ok(())).unwrap();
    let mut command = fixture.command.clone();
    command.operation.as_mut().unwrap().operation_id = uuid::Uuid::new_v4().to_string();
    command.expected_command_sha256 = None;
    let command = plan::prepare(command).unwrap().command;
    let second = workflow::activate(command, |_| Ok(())).unwrap();
    assert_ne!(first.selected_release, second.selected_release);
    assert_ne!(first.command_sha256, second.command_sha256);
    assert!(first.result.unwrap().daemon_path.exists());
}
#[test]
fn changed_plan_digest_is_rejected_before_generation_publication() {
    let fixture = Fixture::new();
    let mut changed = fixture.command.clone();
    changed.expected_command_sha256 = Some("0".repeat(64));
    assert!(workflow::activate(changed, |_| Ok(())).is_err());
    assert!(!fixture.command.runtime_root.join("releases").exists());
    assert!(
        !fixture
            .command
            .runtime_root
            .join("activation-operations")
            .exists()
    );
}
#[test]
fn origin_status_attests_closed_epoch_without_fabricating_owner_or_ack() {
    use gent_protocol::runtime_activation::{
        ActivationOriginLookup, ActivationOriginState as Origin,
    };
    let fixture = Fixture::new();
    fixture.crash("accepted");
    let lookup = fixture.lookup();
    let mut request = ActivationOriginLookup {
        runtime_root: lookup.runtime_root,
        data_directory: lookup.data_directory,
        expected_host_epoch: 1,
    };
    let closed = origin::inspect(&request);
    assert_eq!(closed.state, Origin::Closed);
    assert_eq!(closed.observed_host_epoch, Some(1));
    assert_eq!(closed.observed_closed_ingress, Some(true));
    request.expected_host_epoch = 2;
    assert_eq!(origin::inspect(&request).state, Origin::EpochMismatch);
    let ledger = gent_store::SqliteLedger::open(request.data_directory.join("gent.db")).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    drop(ledger);
    assert_eq!(origin::inspect(&request).state, Origin::NotClosed);
}
#[test]
fn origin_status_missing_scope_creates_nothing_and_held_owner_returns_busy() {
    use fs2::FileExt;
    use gent_protocol::runtime_activation::{
        ActivationOriginLookup, ActivationOriginState as Origin,
    };
    let fixture = Fixture::new();
    fixture.crash("accepted");
    let lookup = fixture.lookup();
    let mut request = ActivationOriginLookup {
        runtime_root: lookup.runtime_root,
        data_directory: lookup.data_directory,
        expected_host_epoch: 1,
    };
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(request.data_directory.join("gentd.lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    assert_eq!(origin::inspect(&request).state, Origin::Busy);
    request.runtime_root = fixture.temp_root.path().join("never-created");
    drop(lock);
    assert_eq!(origin::inspect(&request).state, Origin::Unavailable);
    assert!(!request.runtime_root.exists());
}
#[test]
fn canonical_operation_identity_vectors_for_cross_language_consumers() {
    use gent_protocol::runtime_activation::ActivationOperationPlan;
    let mut digests = Vec::new();
    for suffix in ["ascii", "space #% 日本"] {
        let mut command = ActivationCommand {
            bootstrap_directory: PathBuf::from(format!(r"\\?\C:\Gent QA\{suffix}\bootstrap")),
            runtime_root: PathBuf::from(format!(r"\\?\C:\Gent QA\{suffix}\runtime")),
            data_directory: PathBuf::from(format!(r"\\?\C:\Gent QA\{suffix}\data")),
            expected_daemon_sha256: Some("a".repeat(64)),
            expected_bootstrap_tree_sha256: Some("b".repeat(64)),
            expected_host_epoch: Some(3),
            operation: Some(ActivationOperationExpectation {
                operation_id: "12345678-1234-4234-8234-123456789abc".into(),
                expected_owner: ExpectedActivationOwner {
                    process_id: 3988,
                    executable_digest_sha256: "c".repeat(64),
                },
            }),
            expected_command_sha256: None,
            maintenance_daemon_sha256: "d".repeat(64),
            enable_automatic_updates: false,
        };
        let platform = "x86_64-pc-windows-msvc";
        let version = "v0.2.4";
        let digest = identity::fingerprint(&command, platform, version).unwrap();
        command.expected_command_sha256 = Some(digest.clone());
        assert_eq!(
            identity::fingerprint(&command, platform, version).unwrap(),
            digest
        );
        let plan = ActivationOperationPlan {
            schema: 1,
            operation_id: command.operation.as_ref().unwrap().operation_id.clone(),
            command_sha256: digest.clone(),
            command,
            platform: platform.into(),
            version: version.into(),
            selected_release: format!("{version}+{}.{}-{platform}", "b".repeat(16), digest),
        };
        println!(
            "ACTIVATION_PLAN_VECTOR {}",
            serde_json::to_string(&plan).unwrap()
        );
        digests.push(digest);
    }
    assert_ne!(digests[0], digests[1]);
}

#[cfg(windows)]
#[test]
fn lookup_accepts_drive_namespace_and_separator_variants_without_changing_identity() {
    use gent_protocol::runtime_activation::{
        ActivationOriginLookup, ActivationOriginState as Origin,
    };
    let fixture = Fixture::new();
    fixture.crash("pointerPublished");
    let exact = fixture.lookup();
    for forward in [false, true] {
        let normal = |path: &std::path::Path| {
            let value = path.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
            PathBuf::from(if forward {
                value.replace('\\', "/")
            } else {
                value.to_owned()
            })
        };
        let lookup = ActivationLookup {
            runtime_root: normal(&exact.runtime_root),
            data_directory: normal(&exact.data_directory),
            ..exact.clone()
        };
        let observed = status::inspect(&lookup).unwrap();
        assert_eq!(
            observed.inspection.state,
            Observed::PublishedRecoveryRequired
        );
        assert_eq!(observed.operation.command_sha256, exact.command_sha256);
        let origin = origin::inspect(&ActivationOriginLookup {
            runtime_root: lookup.runtime_root,
            data_directory: lookup.data_directory,
            expected_host_epoch: 1,
        });
        assert_eq!(origin.state, Origin::Closed);
        assert_eq!(origin.runtime_root, exact.runtime_root);
        assert_eq!(origin.data_directory, exact.data_directory);
    }
}

#[test]
fn pointer_read_failure_after_publication_never_claims_prepublication_failure() {
    let fixture = Fixture::new();
    let root = &fixture.command.runtime_root;
    let failure = workflow::activate(fixture.command.clone(), |checkpoint| {
        if checkpoint == "pointerPublished" {
            let pointer = root.join(if cfg!(windows) {
                "current.json"
            } else {
                "current"
            });
            fs::rename(&pointer, root.join("pointer-before-fixture-read-failure")).unwrap();
            fs::write(pointer, "invalid-pointer-fixture").unwrap();
            return Err("synthetic pointer read failure after publication".into());
        }
        Ok(())
    })
    .unwrap_err();
    assert!(failure.contains("synthetic pointer read failure after publication"));
    assert!(super::super::selected_release(root).is_err());
    let lookup = fixture.lookup();
    let stored = journal::read(&lookup.runtime_root, &lookup.operation_id).unwrap();
    assert_eq!(stored.state, State::Uncertain);
    assert!(stored.result.is_none());
    let observed = status::inspect(&lookup).unwrap();
    assert_eq!(observed.operation, stored);
    assert_eq!(observed.inspection.state, Observed::Unavailable);
    assert!(!observed.inspection.adoption_eligible);
}

#[test]
fn positively_preselection_failure_is_recorded_without_publication_claim() {
    let fixture = Fixture::new();
    workflow::activate(fixture.command.clone(), |checkpoint| {
        if checkpoint == "preparing" {
            Err("preselection fixture failure".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    let lookup = fixture.lookup();
    let stored = journal::read(&lookup.runtime_root, &lookup.operation_id).unwrap();
    assert_eq!(stored.state, State::FailedBeforePublication);
    assert!(stored.result.is_none());
    assert!(!lookup.runtime_root.join("releases").exists());
}

#[test]
fn completed_history_survives_active_successor_and_advanced_epoch() {
    let fixture = Fixture::new();
    let completed = workflow::activate(fixture.command.clone(), |_| Ok(())).unwrap();
    let lookup = fixture.lookup();
    let ledger = gent_store::SqliteLedger::open(lookup.data_directory.join("gent.db")).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    drop(ledger);
    let successor = crate::host_lock::acquire(&lookup.data_directory).unwrap();
    let active = status::inspect(&lookup).unwrap();
    assert_eq!(active.operation, completed);
    assert_eq!(active.inspection.state, Observed::Busy);
    assert!(active.operation.result.is_some());
    assert!(!active.inspection.adoption_eligible);
    assert!(status::adopt(&lookup).is_err());
    drop(successor);
    let advanced = status::inspect(&lookup).unwrap();
    assert_eq!(advanced.operation, completed);
    assert_eq!(advanced.inspection.state, Observed::EpochMismatch);
    assert_eq!(advanced.inspection.observed_host_epoch, Some(2));
    assert_eq!(
        advanced.operation.result.unwrap().origin_host_epoch,
        Some(1)
    );
    assert!(!advanced.inspection.adoption_eligible);
}

#[test]
fn unknown_competing_owner_cannot_turn_completed_history_into_current_owner_proof() {
    let fixture = Fixture::new();
    let completed = workflow::activate(fixture.command.clone(), |_| Ok(())).unwrap();
    let lookup = fixture.lookup();
    let _unknown = crate::host_lock::acquire(&lookup.data_directory).unwrap();
    let observed = status::inspect(&lookup).unwrap();
    assert_eq!(observed.operation, completed);
    assert_eq!(observed.inspection.state, Observed::Busy);
    assert_eq!(observed.inspection.observed_host_epoch, None);
    assert_eq!(observed.inspection.selected_release, None);
    assert!(!observed.inspection.adoption_eligible);
    assert!(status::adopt(&lookup).is_err());
}

#[test]
fn delivery_error_after_terminal_persistence_preserves_actual_completed_fact() {
    let fixture = Fixture::new();
    let failure = workflow::activate(fixture.command.clone(), |checkpoint| {
        if checkpoint == "completedPersisted" {
            Err("synthetic lost terminal reply".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(failure.contains("synthetic lost terminal reply"));
    let lookup = fixture.lookup();
    let stored = journal::read(&lookup.runtime_root, &lookup.operation_id).unwrap();
    assert_eq!(stored.state, State::Completed);
    assert!(stored.result.is_some());
    assert_eq!(status::inspect(&lookup).unwrap().operation, stored);
}
