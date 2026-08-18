use std::{
    fs,
    sync::{Arc, Mutex},
};

use gent_drivers::installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix};
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use gent_protocol::DependencyProvider;
use gent_types::{HostEpoch, Receipt, ReceiptId, ReceiptStatus};

use super::{
    PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
    ProvisionReceiptReader, ProvisionedProviderLock, ProvisionedProviderVerifier,
};
use crate::node_runtime_lock::AppNodeRuntimeLock;

#[derive(Clone, Default)]
struct Installer(Arc<Mutex<u8>>);

impl DependencyInstaller for Installer {
    fn install(
        &self,
        _: &NpmGlobalPrefix,
        _: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.0.lock().unwrap() += 1;
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
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
        })
    }
}

#[derive(Clone)]
struct Verifier;

impl ProvisionedProviderVerifier for Verifier {
    fn lock(
        &self,
        _: DependencyProvider,
        _: &std::path::Path,
    ) -> Result<ProvisionedProviderLock, String> {
        panic!("receipt failures must not verify")
    }
}

#[derive(Clone)]
enum ReceiptReader {
    Different,
    Unavailable,
}

impl ProvisionReceiptReader for ReceiptReader {
    fn accepted_receipt(
        &self,
        receipt_id: &ReceiptId,
        idempotency_key: &str,
        host_epoch: HostEpoch,
    ) -> Result<Receipt, String> {
        match self {
            Self::Different => Ok(Receipt {
                receipt_id: receipt_id.clone(),
                idempotency_key: idempotency_key.into(),
                status: ReceiptStatus::Accepted,
                host_epoch: HostEpoch(host_epoch.0 + 1),
            }),
            Self::Unavailable => Err("receipt ledger unavailable".into()),
        }
    }
}

#[test]
fn changed_durable_receipt_binding_refuses_before_npm_effect() {
    let installer = Installer::default();
    let provisioner = provisioner(installer.clone(), ReceiptReader::Different);
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::ReceiptMismatch)
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn unavailable_durable_receipt_refuses_before_npm_effect() {
    let installer = Installer::default();
    let provisioner = provisioner(installer.clone(), ReceiptReader::Unavailable);
    assert!(matches!(
        provisioner.provision(&request()),
        Err(PrivateProvisionError::ReceiptUnavailable(_))
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

fn provisioner(
    installer: Installer,
    receipts: ReceiptReader,
) -> PrivateProviderProvisioner<Installer, Policy, Verifier, ReceiptReader> {
    PrivateProviderProvisioner::new(runtime(), installer, Policy, Some(Verifier), receipts)
}

fn request() -> PrivateProvisionRequest {
    PrivateProvisionRequest {
        receipt: Receipt {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "key".into(),
            status: ReceiptStatus::Accepted,
            host_epoch: HostEpoch(4),
        },
        provider: DependencyProvider::Codex,
        consent_granted: true,
        now_unix_seconds: 1,
    }
}

fn runtime() -> AppNodeRuntimeLock {
    let root = tempfile::tempdir().unwrap().keep();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    AppNodeRuntimeLock::capture(Some(node.into_os_string()), &root.join(".gentd")).unwrap()
}

#[cfg(windows)]
const fn npm_name() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
const fn npm_name() -> &'static str {
    "npm"
}
