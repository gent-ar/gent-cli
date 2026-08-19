use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use gent_protocol::{DependencyProvider, ProviderInstallReview};

use super::install_review;
use crate::{authority_clock::AuthorityClock, dependency_catalog::DependencyCatalog};

#[derive(Clone, Copy)]
struct Clock;

impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        7
    }
}

#[derive(Clone)]
struct Policy {
    version: &'static str,
}

impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(ApprovedPackageInstall {
            provider: provider.into(),
            package_name: "@openai/codex".into(),
            version: self.version.into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        })
    }
}

#[test]
fn policy_artifact_change_produces_a_different_exact_review() {
    let catalog = DependencyCatalog::default();
    let first = review(&catalog, "1.0.0");
    let second = review(&catalog, "2.0.0");

    assert!(first.is_valid());
    assert!(second.is_valid());
    assert_ne!(first.reviewed_plan_digest, second.reviewed_plan_digest);
    assert_eq!(first.package.version, "1.0.0");
}

fn review(catalog: &DependencyCatalog, version: &'static str) -> ProviderInstallReview {
    install_review(
        catalog,
        &Policy { version },
        &Clock,
        DependencyProvider::Codex,
    )
    .unwrap()
}
