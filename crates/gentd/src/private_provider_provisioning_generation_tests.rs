use super::*;

#[derive(Clone, Default)]
struct Attempts(Arc<Mutex<Vec<PathBuf>>>);

impl DependencyInstaller for Attempts {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        _: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        let mut attempts = self.0.lock().unwrap();
        attempts.push(npm.prefix().to_owned());
        let bin = npm.prefix().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("codex"), "installed provider").unwrap();
        if attempts.len() == 1 {
            Err(InstallerError::Failed(
                "effect happened before failure".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
fn explicit_new_receipt_uses_a_clean_attempt_without_replaying_the_old_prefix() {
    let (runtime, _) = runtime();
    let legacy = runtime.rechecked_npm_prefix().unwrap().prefix().to_owned();
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("previous"), "old ambiguous installation").unwrap();
    let installer = Attempts::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier::valid()),
        TestAcceptedReceiptReader,
    );
    let first = request();
    assert!(matches!(
        provisioner.provision(&first),
        Err(PrivateProvisionError::Installer(_))
    ));
    assert!(matches!(
        provisioner.provision(&first),
        Err(PrivateProvisionError::Generation(_))
    ));
    assert_eq!(installer.0.lock().unwrap().len(), 1);
    let mut next = first.clone();
    next.receipt.receipt_id = ReceiptId("explicit-new-receipt".into());
    next.receipt.idempotency_key = "explicit-new-key".into();
    let PrivateProvisionResult::Installed(installation) = provisioner.provision(&next).unwrap()
    else {
        panic!("a new consented attempt must be independently verifiable");
    };
    let attempts = installer.0.lock().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_ne!(attempts[0], attempts[1]);
    assert_eq!(
        fs::read_to_string(attempts[0].join("bin/codex")).unwrap(),
        "installed provider"
    );
    assert_eq!(
        fs::read_to_string(legacy.join("previous")).unwrap(),
        "old ambiguous installation"
    );
    let expected = crate::standalone_provider_setup::provider_generation_prefix(
        &legacy,
        &installation.provenance.receipt_fingerprint_sha256,
    )
    .unwrap()
    .canonicalize()
    .unwrap();
    assert_eq!(attempts[1], expected);
    assert!(PathBuf::from(installation.lock.run_lock.canonical_path).starts_with(expected));
}

#[test]
fn an_existing_empty_attempt_is_not_reused_or_removed() {
    let (runtime, _) = runtime();
    let command =
        gent_runtime::dependency_action_command(&gent_protocol::DependencyActionRequest {
            receipt_id: request().receipt.receipt_id,
            idempotency_key: request().receipt.idempotency_key,
            host_epoch: HostEpoch(1),
            provider: DependencyProvider::Codex,
            action: gent_protocol::DependencyAction::Install,
            consent_granted: true,
            reviewed_plan_digest: request().reviewed_plan_digest,
        });
    let legacy = runtime.rechecked_npm_prefix().unwrap().prefix().to_owned();
    let prefix = crate::standalone_provider_setup::provider_generation_prefix(
        &legacy,
        &command.receipt_fingerprint_sha256(),
    )
    .unwrap();
    fs::create_dir_all(prefix.parent().unwrap()).unwrap();
    assert!(super::super::generation::fresh(&runtime, &command).is_err());
    assert!(prefix.parent().unwrap().is_dir());
    assert!(!prefix.exists());
}

#[test]
fn a_generation_root_that_is_a_file_is_rejected_without_installer_effect() {
    let (runtime, _) = runtime();
    let legacy = runtime.rechecked_npm_prefix().unwrap().prefix().to_owned();
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(
        legacy.parent().unwrap().join("generations"),
        "not a directory",
    )
    .unwrap();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier::valid()),
        TestAcceptedReceiptReader,
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::Generation(_))
    ));
    assert_eq!(*installer.calls.lock().unwrap(), 0);
}
