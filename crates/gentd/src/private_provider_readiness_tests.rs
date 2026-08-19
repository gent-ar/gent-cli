use gent_drivers::lock::capture;
use gent_ports::{LedgerError, ProvisionedProviderLockReader};
use gent_types::{
    AgentChatProvider, ProviderInstallProvenance, ProvisionedProviderInstallation,
    ProvisionedProviderLock,
};

use super::{PrivateProviderReadiness, PrivateProviderReadinessService};

#[derive(Clone, Debug)]
enum Reader {
    Available(Box<Option<ProvisionedProviderInstallation>>),
    Unavailable,
}

impl ProvisionedProviderLockReader for Reader {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        match self {
            Self::Available(installation) => Ok(*installation.clone()),
            Self::Unavailable => Err(LedgerError::Storage("unavailable".into())),
        }
    }
}

#[test]
fn missing_lock_yields_a_daemon_generated_install_review() {
    let readiness = service(Reader::Available(Box::new(None))).assess(AgentChatProvider::Claude);

    assert_eq!(readiness, PrivateProviderReadiness::InstallReview);
}

#[test]
fn changed_lock_requires_a_new_install_review_without_path_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    std::fs::write(&executable, "locked provider").unwrap();
    let lock = capture("codex", &executable, "1.0", "entry").unwrap();
    std::fs::write(executable, "changed provider").unwrap();

    let readiness = service(Reader::Available(Box::new(Some(installation(lock)))))
        .assess(AgentChatProvider::Codex);

    assert_eq!(readiness, PrivateProviderReadiness::InvalidInstallation);
}

#[test]
fn claurst_never_enters_the_public_npm_readiness_path() {
    let readiness = service(Reader::Unavailable).assess(AgentChatProvider::Claurst);

    assert_eq!(readiness, PrivateProviderReadiness::ClaurstUnavailable);
}

#[test]
fn unreadable_provenance_never_proposes_an_install() {
    let readiness = service(Reader::Unavailable).assess(AgentChatProvider::Codex);

    assert_eq!(readiness, PrivateProviderReadiness::Unavailable);
}

fn service(reader: Reader) -> PrivateProviderReadinessService<Reader> {
    PrivateProviderReadinessService::new(reader)
}

fn installation(lock: gent_types::RunVersionLock) -> ProvisionedProviderInstallation {
    ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock { run_lock: lock },
        provenance: ProviderInstallProvenance {
            package_name: "package".into(),
            package_version: "1.0".into(),
            package_integrity: "integrity".into(),
            package_policy_digest_sha256: "a".repeat(64),
            node_runtime_digest_sha256: "b".repeat(64),
            release_artifact_digest_sha256: "c".repeat(64),
            receipt_fingerprint_sha256: "d".repeat(64),
        },
    }
}
