use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gent_drivers::installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix};
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use gent_protocol::DependencyProvider;
use gent_types::{HostEpoch, Receipt, ReceiptId, ReceiptStatus};

use super::{
    PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
    PrivateProvisionResult, ProvisionedProviderLock, ProvisionedProviderVerifier,
};
use crate::node_runtime_lock::AppNodeRuntimeLock;

#[derive(Clone, Default)]
struct Installer {
    calls: Arc<Mutex<u8>>,
    mutate: Option<PathBuf>,
}

impl DependencyInstaller for Installer {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.calls.lock().unwrap() += 1;
        let bin = npm.prefix().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(&package.provider), "installed provider").unwrap();
        if let Some(path) = &self.mutate {
            fs::write(path, "changed after install").unwrap();
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Policy;

impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(ApprovedPackageInstall {
            provider: provider.into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
        })
    }
}

#[derive(Clone)]
struct Verifier {
    unsupported: bool,
    changed: bool,
}

impl Verifier {
    const fn valid() -> Self {
        Self {
            unsupported: false,
            changed: false,
        }
    }
}

impl ProvisionedProviderVerifier for Verifier {
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &std::path::Path,
    ) -> Result<ProvisionedProviderLock, String> {
        if self.unsupported {
            return Err("unsupported installed binary".into());
        }
        Ok(ProvisionedProviderLock {
            provider,
            executable: prefix.join("bin").join(if self.changed {
                "substituted"
            } else {
                provider.as_str()
            }),
            version: "1.0.0".into(),
            digest_sha256: "a".repeat(64),
        })
    }
}

#[test]
fn consent_is_required_before_policy_or_npm_effects() {
    let (runtime, _) = runtime();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier::valid()),
    );
    assert_eq!(
        provisioner
            .provision(&PrivateProvisionRequest {
                consent_granted: false,
                ..request()
            })
            .unwrap(),
        PrivateProvisionResult::ConsentRequired
    );
    assert_eq!(*installer.calls.lock().unwrap(), 0);
}

#[test]
fn changed_node_is_refused_before_the_fixed_installer_effect() {
    let (runtime, node) = runtime();
    fs::write(node, "substituted").unwrap();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier::valid()),
    );
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::Runtime(_))
    ));
    assert_eq!(*installer.calls.lock().unwrap(), 0);
}

#[test]
fn post_effect_runtime_change_is_ambiguous_and_never_claimed_installed() {
    let (runtime, node) = runtime();
    let installer = Installer {
        calls: Arc::default(),
        mutate: Some(node),
    };
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier::valid()),
    );
    assert_eq!(
        provisioner.provision(&request()).unwrap(),
        PrivateProvisionResult::Ambiguous
    );
    assert_eq!(*installer.calls.lock().unwrap(), 1);
}

#[test]
fn only_accepted_receipts_can_reach_the_private_seam() {
    let (runtime, _) = runtime();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        Installer::default(),
        Policy,
        Some(Verifier::valid()),
    );
    let mut request = request();
    request.receipt.status = ReceiptStatus::Settled;
    assert!(matches!(
        provisioner.provision(&request),
        Err(PrivateProvisionError::ReceiptNotAccepted)
    ));
}

#[test]
fn missing_post_install_verifier_refuses_before_npm_effects() {
    let (runtime, _) = runtime();
    let installer = Installer::default();
    let provisioner =
        PrivateProviderProvisioner::new(runtime, installer.clone(), Policy, None::<Verifier>);
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::VerificationUnavailable)
    ));
    assert_eq!(*installer.calls.lock().unwrap(), 0);
}

#[test]
fn unsupported_post_install_provider_is_ambiguous_after_the_effect() {
    let (runtime, _) = runtime();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier {
            unsupported: true,
            changed: false,
        }),
    );
    assert_eq!(
        provisioner.provision(&request()).unwrap(),
        PrivateProvisionResult::Ambiguous
    );
    assert_eq!(*installer.calls.lock().unwrap(), 1);
}

#[test]
fn changed_or_missing_post_install_executable_is_ambiguous() {
    let (runtime, _) = runtime();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        Installer::default(),
        Policy,
        Some(Verifier {
            unsupported: false,
            changed: true,
        }),
    );
    assert_eq!(
        provisioner.provision(&request()).unwrap(),
        PrivateProvisionResult::Ambiguous
    );
}

#[test]
fn valid_post_install_executable_version_and_digest_lock_can_settle_installed() {
    let (runtime, _) = runtime();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        Installer::default(),
        Policy,
        Some(Verifier::valid()),
    );
    assert!(matches!(
        provisioner.provision(&request()).unwrap(),
        PrivateProvisionResult::Installed(ProvisionedProviderLock {
            provider: DependencyProvider::Codex,
            ..
        })
    ));
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
        consent_granted: true,
        now_unix_seconds: 1,
    }
}

fn runtime() -> (AppNodeRuntimeLock, PathBuf) {
    let root = tempfile::tempdir().unwrap().keep();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    (
        AppNodeRuntimeLock::capture(Some(node.clone().into_os_string()), &root.join(".gentd"))
            .unwrap(),
        node,
    )
}

#[cfg(windows)]
const fn npm_name() -> &'static str {
    "npm.cmd"
}
#[cfg(not(windows))]
const fn npm_name() -> &'static str {
    "npm"
}
