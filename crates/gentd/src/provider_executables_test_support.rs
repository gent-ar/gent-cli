use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use ed25519_dalek::SigningKey;
use gent_drivers::lock::capture;
use gent_ports::{LedgerError, ProvisionedProviderLockReader};
use gent_protocol::DependencyProvider;
use gent_types::{
    AgentChatProvider, ProviderInstallProvenance, ProvisionedProviderInstallation,
    ProvisionedProviderLock, RunVersionLock,
};

use super::ProviderExecutables;
use crate::{
    ordinary_authority_release::fixture,
    private_provider_readiness::PrivateProviderReadiness,
    standalone_authority_release::StandaloneAuthorityRelease,
    standalone_provider_setup::{provider_executable, provider_prefix},
};

pub(super) const VERSION: &str = "codex-cli 0.153.4";

#[derive(Clone)]
pub(super) struct Installed(pub(super) Arc<Mutex<Result<Option<RunVersionLock>, ()>>>);

impl ProvisionedProviderLockReader for Installed {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        let lock = self.0.lock().unwrap().clone();
        lock.map_err(|()| LedgerError::Storage("unreadable".into()))
            .map(|lock| {
                lock.map(|run_lock| ProvisionedProviderInstallation {
                    lock: ProvisionedProviderLock { run_lock },
                    provenance: ProviderInstallProvenance {
                        package_name: "@openai/codex".into(),
                        package_version: "0.153.4-darwin-arm64".into(),
                        package_integrity: "sha512-test".into(),
                        package_policy_digest_sha256: "a".repeat(64),
                        node_runtime_digest_sha256: "b".repeat(64),
                        release_artifact_digest_sha256: "c".repeat(64),
                        receipt_fingerprint_sha256: "d".repeat(64),
                    },
                })
            })
    }
}

pub(super) struct Scenario {
    pub(super) directory: tempfile::TempDir,
    pub(super) signer: SigningKey,
    pub(super) release: StandaloneAuthorityRelease,
    pub(super) installed: Installed,
}

impl Scenario {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = fixture::runtime(&directory.path().join("runtime"));
        let signer = SigningKey::from_bytes(&[21; 32]);
        let release = StandaloneAuthorityRelease::configured(
            directory.path().join("authority.json"),
            &[format!(
                "root:{}",
                hex::encode(signer.verifying_key().as_bytes())
            )],
            runtime,
        )
        .unwrap();
        Self {
            directory,
            signer,
            release,
            installed: Installed(Arc::new(Mutex::new(Ok(None)))),
        }
    }

    pub(super) fn native_path(&self) -> PathBuf {
        provider_executable(
            &provider_prefix(self.directory.path()),
            DependencyProvider::Codex,
        )
        .unwrap()
    }

    pub(super) fn install_at(&self, executable: &Path, bytes: &str) -> RunVersionLock {
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(executable, bytes).unwrap();
        let lock = capture("codex", executable, VERSION, "codex-0.1.0").unwrap();
        *self.installed.0.lock().unwrap() = Ok(Some(lock.clone()));
        lock
    }

    pub(super) fn sign(&self, digest: &str, revoked: bool) {
        let node = self
            .release
            .runtime()
            .unwrap()
            .node_digest_sha256()
            .to_owned();
        let release = if revoked {
            fixture::revoked_release(&self.signer, &node)
        } else {
            fixture::release_for_codex(&self.signer, &node, VERSION, digest)
        };
        fs::write(
            self.directory.path().join("authority.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
    }

    pub(super) fn executables(&self, release: bool) -> ProviderExecutables {
        ProviderExecutables::standalone(
            None,
            None,
            self.directory.path(),
            self.installed.clone(),
            release.then(|| self.release.clone()),
        )
    }

    pub(super) fn signed_install(&self) -> (RunVersionLock, ProviderExecutables) {
        let lock = self.install_at(&self.native_path(), "native codex");
        self.sign(&lock.digest_sha256, false);
        (lock, self.executables(true))
    }
}

pub(super) fn readiness(executables: &ProviderExecutables) -> PrivateProviderReadiness {
    executables.installed_readiness(AgentChatProvider::Codex)
}
