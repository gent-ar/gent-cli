use std::{
    fs,
    sync::{Arc, Mutex},
};

use ed25519_dalek::SigningKey;
use gent_drivers::installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix};
use gent_ports::ApprovedPackageInstall;
use gent_protocol::DependencyProvider;
use gent_types::{HostEpoch, ReceiptId};
use sha2::Digest;

use super::receipt_tests::{
    Policy, ReceiptReader, prompt_binding_with_release_digest, request, runtime_at,
};
use super::{
    PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionResult,
    ProvisionedProviderLock, ProvisionedProviderVerifier, ReleaseAuthorityConfig,
};
use crate::ordinary_authority_release::fixture::{release, revoked_release, root_keys};

#[derive(Clone, Default)]
struct InstallingInstaller(Arc<Mutex<u8>>);

impl DependencyInstaller for InstallingInstaller {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.0.lock().unwrap() += 1;
        let bin = npm.prefix().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(&package.provider), "installed provider").unwrap();
        Ok(())
    }
}

#[derive(Clone)]
struct ValidVerifier;

impl ProvisionedProviderVerifier for ValidVerifier {
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &std::path::Path,
    ) -> Result<ProvisionedProviderLock, String> {
        gent_drivers::lock::capture(
            provider.as_str(),
            &prefix.join("bin").join(provider.as_str()),
            "1.0.0",
            "test",
        )
        .map(|run_lock| ProvisionedProviderLock { run_lock })
        .map_err(|error| error.to_string())
    }
}

#[test]
fn changed_release_artifact_digest_refuses_before_npm_effect() {
    let root = tempfile::tempdir().unwrap().keep();
    let runtime = runtime_at(&root);
    let signer = SigningKey::from_bytes(&[9; 32]);
    let envelope = release(&signer, runtime.node_digest_sha256());
    let release_path = root.join("ordinary-authority.json");
    fs::write(&release_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let installer = InstallingInstaller::default();
    let provisioner = PrivateProviderProvisioner::with_release_authority(
        runtime,
        installer.clone(),
        Policy,
        Some(ValidVerifier),
        ReceiptReader::Accepted,
        ReleaseAuthorityConfig {
            path: release_path,
            root_keys: root_keys(&signer),
        },
    );
    let binding = prompt_binding_with_release_digest("1.0.0", &"f".repeat(64));
    let command = gent_runtime::prompt_provider_provision_command(
        ReceiptId("receipt".into()),
        "key".into(),
        HostEpoch(4),
        &binding,
    );
    assert!(matches!(
        provisioner.provision_prompt_with_command(&request(), &command, &binding),
        Err(PrivateProvisionError::ReleaseDigestMismatch)
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn revoked_release_refuses_before_npm_effect() {
    let root = tempfile::tempdir().unwrap().keep();
    let runtime = runtime_at(&root);
    let signer = SigningKey::from_bytes(&[10; 32]);
    let envelope = revoked_release(&signer, runtime.node_digest_sha256());
    let release_path = root.join("ordinary-authority.json");
    fs::write(&release_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let installer = InstallingInstaller::default();
    let provisioner = PrivateProviderProvisioner::with_release_authority(
        runtime,
        installer.clone(),
        Policy,
        Some(ValidVerifier),
        ReceiptReader::Accepted,
        ReleaseAuthorityConfig {
            path: release_path,
            root_keys: root_keys(&signer),
        },
    );
    let binding = prompt_binding_with_release_digest("1.0.0", &"f".repeat(64));
    let command = gent_runtime::prompt_provider_provision_command(
        ReceiptId("receipt".into()),
        "key".into(),
        HostEpoch(4),
        &binding,
    );
    assert!(matches!(
        provisioner.provision_prompt_with_command(&request(), &command, &binding),
        Err(PrivateProvisionError::ReleaseReauthorizationFailed(_))
    ));
    assert_eq!(*installer.0.lock().unwrap(), 0);
}

#[test]
fn matching_release_digest_reaches_installed() {
    let root = tempfile::tempdir().unwrap().keep();
    let runtime = runtime_at(&root);
    let signer = SigningKey::from_bytes(&[11; 32]);
    let envelope = release(&signer, runtime.node_digest_sha256());
    let release_path = root.join("ordinary-authority.json");
    fs::write(&release_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let expected_digest = hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(&serde_json::to_value(&envelope).unwrap()).unwrap(),
    ));
    let installer = InstallingInstaller::default();
    let provisioner = PrivateProviderProvisioner::with_release_authority(
        runtime,
        installer.clone(),
        Policy,
        Some(ValidVerifier),
        ReceiptReader::Accepted,
        ReleaseAuthorityConfig {
            path: release_path,
            root_keys: root_keys(&signer),
        },
    );
    let binding = prompt_binding_with_release_digest("1.0.0", &expected_digest);
    let command = gent_runtime::prompt_provider_provision_command(
        ReceiptId("receipt".into()),
        "key".into(),
        HostEpoch(4),
        &binding,
    );
    let result = provisioner
        .provision_prompt_with_command(&request(), &command, &binding)
        .unwrap();
    let PrivateProvisionResult::Installed(installation) = result else {
        panic!("expected an installed result, got {result:?}");
    };
    assert_eq!(
        installation.provenance.release_artifact_digest_sha256,
        expected_digest
    );
    assert_eq!(
        installation.provenance.receipt_fingerprint_sha256,
        command.receipt_fingerprint_sha256()
    );
    assert_eq!(*installer.0.lock().unwrap(), 1);
}
