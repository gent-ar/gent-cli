//! Exact public package review derived from one signed policy at daemon authority.

use gent_ports::PackageInstallPolicy;
use gent_protocol::{
    DependencyAction, DependencyPlanRequest, DependencyProvider, ProviderInstallReview,
    ProviderPackageReview,
};

use crate::{authority_clock::AuthorityClock, dependency_catalog::DependencyCatalog};

/// Derives the sole public install review shared by readiness and prompt provisioning.
///
/// The caller supplies no package fields. This function binds Gent's stable instruction to the
/// currently approved package policy before either client review or receipt admission occurs.
pub(crate) fn install_review<P: PackageInstallPolicy, C: AuthorityClock>(
    catalog: &DependencyCatalog,
    policy: &P,
    clock: &C,
    provider: DependencyProvider,
) -> Result<ProviderInstallReview, String> {
    let plan = catalog.plan(DependencyPlanRequest {
        provider,
        action: DependencyAction::Install,
    });
    let package = policy
        .approved_package(provider.as_str(), clock.now_unix_seconds())
        .map_err(|error| error.to_string())?;
    if package.provider != provider.as_str() {
        return Err("package policy selected a different provider".into());
    }
    let review = ProviderInstallReview::reviewed(
        provider,
        DependencyAction::Install,
        plan.instruction,
        plan.consent_required,
        ProviderPackageReview {
            package_name: package.package_name,
            version: package.version,
            integrity: package.integrity,
            package_policy_digest_sha256: package.package_policy_digest_sha256,
        },
    );
    review
        .is_valid()
        .then_some(review)
        .ok_or_else(|| "signed package policy produced an invalid public review".into())
}

#[cfg(test)]
#[path = "private_provider_review_tests.rs"]
mod tests;
