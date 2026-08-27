use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use gent_drivers::{
    installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix},
    lock::capture,
};
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use gent_protocol::DependencyProvider;
use gent_types::{HostEpoch, Receipt, ReceiptId, ReceiptStatus, RunVersionLock};

use super::{
    PrivateProviderProvisioner, PrivateProvisionRequest, PrivateProvisionResult,
    ProvisionedProviderLock, ProvisionedProviderVerifier, TestAcceptedReceiptReader,
};
use crate::{
    node_runtime_lock::AppNodeRuntimeLock,
    private_provider_compatibility::ProvisionedProviderCompatibility,
};

#[derive(Clone, Default)]
struct Installer(Arc<Mutex<u8>>);

impl DependencyInstaller for Installer {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.0.lock().unwrap() += 1;
        let bin = npm.prefix().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(&package.provider), "provider").unwrap();
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
            package_policy_digest_sha256: "a".repeat(64),
        })
    }
}

#[derive(Clone)]
struct Verifier;

impl ProvisionedProviderVerifier for Verifier {
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &Path,
    ) -> Result<ProvisionedProviderLock, String> {
        capture(
            provider.as_str(),
            &prefix.join("bin").join(provider.as_str()),
            "1.0.0",
            "unbound",
        )
        .map(|run_lock| ProvisionedProviderLock { run_lock })
        .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
struct DeniedCompatibility;

impl ProvisionedProviderCompatibility for DeniedCompatibility {
    fn bind(&self, _: RunVersionLock, _: u64) -> Result<RunVersionLock, String> {
        Err("signed compatibility evidence rejected the observed lock".into())
    }
}

#[test]
fn rejected_post_install_compatibility_is_ambiguous_and_never_creates_a_lock() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    let npm_cli = root.path().join("lib/node_modules/npm/bin");
    fs::create_dir_all(&npm_cli).unwrap();
    fs::write(npm_cli.join("npm-cli.js"), "npm cli").unwrap();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::with_compatibility(
        AppNodeRuntimeLock::capture(Some(node.into_os_string()), root.path()).unwrap(),
        installer.clone(),
        Policy,
        Some(Verifier),
        TestAcceptedReceiptReader,
        DeniedCompatibility,
    );

    let result = provisioner.provision(&request()).unwrap();

    assert_eq!(result, PrivateProvisionResult::Ambiguous);
    assert_eq!(*installer.0.lock().unwrap(), 1);
}

fn request() -> PrivateProvisionRequest {
    PrivateProvisionRequest {
        receipt: Receipt {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "key".into(),
            status: ReceiptStatus::Accepted,
            host_epoch: HostEpoch(1),
        },
        provider: DependencyProvider::Codex,
        action: gent_protocol::DependencyAction::Install,
        reviewed_plan_digest: "plan".into(),
        consent_granted: true,
        now_unix_seconds: 1,
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
