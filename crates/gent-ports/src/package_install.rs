//! Trusted, exact package coordinates selected only by daemon-owned policy.

/// An immutable npm package artifact authorized for one public provider.
///
/// The package name, exact version, and SRI integrity are policy data. Clients
/// and provider prompts never supply any of these values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedPackageInstall {
    pub provider: String,
    pub package_name: String,
    pub version: String,
    pub integrity: String,
}

impl ApprovedPackageInstall {
    /// Returns the exact npm package selector; no range or tag is permitted.
    #[must_use]
    pub fn selector(&self) -> String {
        format!("{}@{}", self.package_name, self.version)
    }
}

/// Failure to obtain a trusted package coordinate for a provider install.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackageInstallPolicyError {
    #[error("no trusted package policy is available for provider {provider}")]
    Unavailable { provider: String },
    #[error("trusted package policy rejected provider {provider}: {reason}")]
    Rejected { provider: String, reason: String },
}

/// Resolves a public provider to one exact, integrity-pinned package artifact.
pub trait PackageInstallPolicy: Send + Sync {
    /// Selects a package only after the policy's signature, expiry, and runtime
    /// binding have been checked by its daemon-owned implementation.
    ///
    /// # Errors
    /// Returns an error when no package is currently approved for the provider.
    fn approved_package(
        &self,
        provider: &str,
        now_unix_seconds: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError>;
}

#[cfg(test)]
mod tests {
    use super::ApprovedPackageInstall;

    #[test]
    fn selector_is_exact_and_never_a_latest_tag() {
        let package = ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.147.0".into(),
            integrity: "sha512-test".into(),
        };
        assert_eq!(package.selector(), "@openai/codex@0.147.0");
    }
}
