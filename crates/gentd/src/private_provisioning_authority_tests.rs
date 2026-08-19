use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use gent_drivers::{
    installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix},
    lock::capture,
};
use gent_ports::{
    ApprovedPackageInstall, Ledger, PackageInstallPolicy, PackageInstallPolicyError,
    ProvisionedProviderLockLedger, ReceiptClaim,
};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionState, DependencyPlanRequest,
    DependencyProvider,
};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, ReceiptId};

use super::PrivateProvisioningAuthority;
use crate::authority_clock::AuthorityClock;
use crate::{
    compatibility_assessment::CompatibilityAssessment,
    dependency_catalog::DependencyCatalog,
    node_runtime_lock::AppNodeRuntimeLock,
    private_provider_provisioning::{
        PrivateProviderProvisioner, ProvisionedProviderLock, ProvisionedProviderVerifier,
        TestAcceptedReceiptReader,
    },
};

type Authority = PrivateProvisioningAuthority<
    SqliteLedger,
    Installer,
    Policy,
    Verifier,
    TestAcceptedReceiptReader,
    crate::private_provider_compatibility::TestProvisionedProviderCompatibility,
    Clock,
>;

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
            "test",
        )
        .map(|run_lock| ProvisionedProviderLock { run_lock })
        .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy)]
struct Clock;

impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        1
    }
}

#[test]
fn denied_consent_settles_without_starting_npm() {
    let (authority, installer, _, _root) = authority();
    let mut request = request("denied", "wrong");
    request.consent_granted = false;

    let result = authority.execute(&request).unwrap();

    assert_eq!(result.state, DependencyActionState::ConsentRequired);
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn mismatched_daemon_plan_is_rejected_without_starting_npm() {
    let (authority, installer, _, _root) = authority();

    let result = authority
        .execute(&request("mismatch", "not-a-daemon-plan"))
        .unwrap();

    assert_eq!(result.state, DependencyActionState::PlanMismatch);
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn verified_installation_settles_the_receipt_with_a_durable_lock() {
    let (authority, installer, ledger, _root) = authority();
    let plan = DependencyCatalog::with_compatibility(CompatibilityAssessment::default()).plan(
        DependencyPlanRequest {
            provider: DependencyProvider::Codex,
            action: DependencyAction::Install,
        },
    );

    let result = authority
        .execute(&request("installed", &plan.reviewed_plan_digest))
        .unwrap();

    assert_eq!(result.state, DependencyActionState::Completed);
    assert_eq!(*installer.0.lock().unwrap(), 1);
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_some()
    );
}

#[test]
fn recovered_accepted_receipt_becomes_unprovable_without_replaying_npm() {
    let (authority, installer, ledger, _root) = authority();
    let plan = DependencyCatalog::with_compatibility(CompatibilityAssessment::default()).plan(
        DependencyPlanRequest {
            provider: DependencyProvider::Codex,
            action: DependencyAction::Install,
        },
    );
    let request = request("recovered", &plan.reviewed_plan_digest);
    let command = gent_runtime::dependency_action_command(&request);
    let accepted = match ledger
        .claim_command(
            &command,
            &gent_types::Event {
                cursor: 0,
                event_id: "accepted-before-restart".into(),
                receipt_id: request.receipt_id.clone(),
                host_epoch: request.host_epoch,
                kind: "dependencyActionAccepted".into(),
                payload: command.payload.clone(),
            },
        )
        .unwrap()
    {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("fixture must claim a new receipt"),
    };

    let result = authority.execute(&request).unwrap();

    assert_eq!(result.state, DependencyActionState::Unprovable);
    assert_eq!(result.receipt.receipt_id, accepted.receipt_id);
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

fn authority() -> (Authority, Installer, SqliteLedger, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    let npm_cli = root.path().join("lib/node_modules/npm/bin");
    fs::create_dir_all(&npm_cli).unwrap();
    fs::write(npm_cli.join("npm-cli.js"), "npm cli").unwrap();
    let runtime = AppNodeRuntimeLock::capture(Some(node.into_os_string()), root.path()).unwrap();
    let installer = Installer::default();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        installer.clone(),
        Policy,
        Some(Verifier),
        TestAcceptedReceiptReader,
    );
    let ledger = SqliteLedger::in_memory().unwrap();
    let authority = PrivateProvisioningAuthority::new(
        ledger.clone(),
        DependencyCatalog::with_compatibility(CompatibilityAssessment::default()),
        provisioner,
        Clock,
    );
    (authority, installer, ledger, root)
}

fn request(key: &str, reviewed_plan_digest: &str) -> DependencyActionRequest {
    DependencyActionRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
        consent_granted: true,
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        reviewed_plan_digest: reviewed_plan_digest.into(),
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
