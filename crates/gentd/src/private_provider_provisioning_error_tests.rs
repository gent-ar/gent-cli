use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use gent_drivers::installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix};
use gent_drivers::lock::capture;
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use gent_protocol::DependencyProvider;
use gent_types::{HostEpoch, Receipt, ReceiptId, ReceiptStatus, RunVersionLock};

use super::{
    PrivateProviderProvisioner, PrivateProvisionError, ProvisionedProviderLock,
    ProvisionedProviderVerifier, TestAcceptedReceiptReader,
};
use crate::{
    node_runtime_lock::AppNodeRuntimeLock, private_provider_provisioning::PrivateProvisionRequest,
};

#[derive(Clone, Default)]
struct Installer(Arc<Mutex<u8>>);

impl DependencyInstaller for Installer {
    fn install(
        &self,
        _: &NpmGlobalPrefix,
        _: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.0.lock().unwrap() += 1;
        Err(InstallerError::Failed("expected failure".into()))
    }
}

#[derive(Clone)]
struct Reject;

impl PackageInstallPolicy for Reject {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Err(PackageInstallPolicyError::Rejected {
            provider: provider.into(),
            reason: "test rejection".into(),
        })
    }
}

#[derive(Clone)]
struct Mismatch;

impl PackageInstallPolicy for Mismatch {
    fn approved_package(
        &self,
        _: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(package("claude"))
    }
}

#[derive(Clone)]
struct GoodPolicy;

impl PackageInstallPolicy for GoodPolicy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(package(provider))
    }
}

#[derive(Clone)]
struct BadDigestPolicy;

impl PackageInstallPolicy for BadDigestPolicy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        let mut package = package(provider);
        package.package_policy_digest_sha256 = "not-a-digest".into();
        Ok(package)
    }
}

#[derive(Clone)]
struct NeverVerifier;

impl ProvisionedProviderVerifier for NeverVerifier {
    fn lock(&self, _: DependencyProvider, _: &Path) -> Result<ProvisionedProviderLock, String> {
        panic!("pre-effect failures must not verify")
    }
}

#[test]
fn rejected_or_mismatched_policy_never_reaches_the_installer() {
    let (temp, runtime) = runtime();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Reject,
        Some(NeverVerifier),
        TestAcceptedReceiptReader,
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::Policy(_))
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
    let provisioner = PrivateProviderProvisioner::new(
        AppNodeRuntimeLock::capture(
            Some(temp.path().join("bin/node").into_os_string()),
            &temp.path().join(".gentd"),
        )
        .unwrap(),
        installer.clone(),
        Mismatch,
        Some(NeverVerifier),
        TestAcceptedReceiptReader,
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::ProviderMismatch)
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn invalid_policy_identity_never_reaches_the_installer() {
    let (_temp, runtime) = runtime();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        BadDigestPolicy,
        Some(NeverVerifier),
        TestAcceptedReceiptReader,
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::PolicyIdentity)
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn installer_failure_is_reported_without_post_effect_verification() {
    let (_temp, runtime) = runtime();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        GoodPolicy,
        Some(NeverVerifier),
        TestAcceptedReceiptReader,
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::Installer(_))
    ));
    assert_eq!(*installer.0.lock().unwrap(), 1);
}

#[test]
fn lock_validation_rejects_escape_empty_version_and_noncanonical_digest() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("prefix");
    let executable = prefix.join("bin/codex");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "codex").unwrap();
    let lock = ProvisionedProviderLock {
        run_lock: capture("codex", &executable, "1.0.0", "test").unwrap(),
    };
    assert!(crate::private_provider_lock_validation::valid_lock(
        &lock,
        DependencyProvider::Codex,
        &prefix
    ));
    for replacement in [
        ProvisionedProviderLock {
            run_lock: RunVersionLock {
                canonical_path: temp.path().join("escape").display().to_string(),
                ..lock.run_lock.clone()
            },
        },
        ProvisionedProviderLock {
            run_lock: RunVersionLock {
                version: " \t".into(),
                ..lock.run_lock.clone()
            },
        },
        ProvisionedProviderLock {
            run_lock: RunVersionLock {
                digest_sha256: "A".repeat(64),
                ..lock.run_lock.clone()
            },
        },
    ] {
        assert!(!crate::private_provider_lock_validation::valid_lock(
            &replacement,
            DependencyProvider::Codex,
            &prefix
        ));
    }
}

fn request() -> PrivateProvisionRequest {
    PrivateProvisionRequest {
        receipt: Receipt {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "receipt-key".into(),
            status: ReceiptStatus::Accepted,
            host_epoch: HostEpoch(1),
        },
        provider: DependencyProvider::Codex,
        action: gent_protocol::DependencyAction::Install,
        reviewed_plan_digest: "reviewed-plan-digest".into(),
        consent_granted: true,
        now_unix_seconds: 1,
    }
}

fn runtime() -> (tempfile::TempDir, AppNodeRuntimeLock) {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    let npm_cli = temp.path().join("lib/node_modules/npm/bin");
    fs::create_dir_all(&npm_cli).unwrap();
    fs::write(npm_cli.join("npm-cli.js"), "npm cli").unwrap();
    let runtime =
        AppNodeRuntimeLock::capture(Some(node.into_os_string()), &temp.path().join(".gentd"))
            .unwrap();
    (temp, runtime)
}

fn package(provider: &str) -> ApprovedPackageInstall {
    ApprovedPackageInstall {
        provider: provider.into(),
        package_name: "package".into(),
        version: "1.0.0".into(),
        integrity: "sha512-test".into(),
        package_policy_digest_sha256: "a".repeat(64),
    }
}

#[cfg(windows)]
const fn npm_name() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
const fn npm_name() -> &'static str {
    "npm"
}
