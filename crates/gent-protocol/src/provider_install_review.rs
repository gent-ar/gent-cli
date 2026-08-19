//! Public, daemon-issued review of one exact public provider package operation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{DependencyAction, DependencyProvider};

/// Exact public npm artifact covered by one provider-install review.
///
/// This type is limited to public package provenance. It contains no executable path, runtime
/// session, private bridge, credential, or routing information.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPackageReview {
    pub package_name: String,
    pub version: String,
    pub integrity: String,
    pub package_policy_digest_sha256: String,
}

/// Human-reviewable, daemon-issued public provider install operation.
///
/// Clients receive this value only from provider readiness and confirm it by echoing its digest;
/// they never supply the provider, package, policy, or plan fields back to Gent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderInstallReview {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
    pub instruction: String,
    pub consent_required: bool,
    pub package: ProviderPackageReview,
    pub reviewed_plan_digest: String,
}

impl ProviderInstallReview {
    /// Builds the digest clients must echo for this exact reviewed public package operation.
    #[must_use]
    pub fn reviewed(
        provider: DependencyProvider,
        action: DependencyAction,
        instruction: impl Into<String>,
        consent_required: bool,
        package: ProviderPackageReview,
    ) -> Self {
        let instruction = instruction.into();
        let reviewed_plan_digest = provider_install_review_digest(
            provider,
            action,
            &instruction,
            consent_required,
            &package,
        );
        Self {
            provider,
            action,
            instruction,
            consent_required,
            package,
            reviewed_plan_digest,
        }
    }

    /// Verifies the public artifact is bounded and its digest covers every review field.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        text(&self.instruction, 2_048)
            && package_text(&self.package.package_name, 214)
            && package_text(&self.package.version, 128)
            && self.package.integrity.starts_with("sha512-")
            && package_text(&self.package.integrity, 512)
            && digest(&self.package.package_policy_digest_sha256)
            && digest(&self.reviewed_plan_digest)
            && self.reviewed_plan_digest
                == provider_install_review_digest(
                    self.provider,
                    self.action,
                    &self.instruction,
                    self.consent_required,
                    &self.package,
                )
    }
}

/// Returns the canonical digest for every user-visible and policy-bound review field.
///
/// # Panics
///
/// Panics only if this fixed serializable protocol DTO cannot be encoded as JSON.
#[must_use]
pub fn provider_install_review_digest(
    provider: DependencyProvider,
    action: DependencyAction,
    instruction: &str,
    consent_required: bool,
    package: &ProviderPackageReview,
) -> String {
    let document = serde_json::json!({
        "action": action,
        "consentRequired": consent_required,
        "instruction": instruction,
        "package": package,
        "provider": provider,
    });
    let bytes = serde_json::to_vec(&document).expect("provider install review serializes");
    hex::encode(Sha256::digest(bytes))
}

fn text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn package_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::{ProviderInstallReview, ProviderPackageReview};
    use crate::{DependencyAction, DependencyProvider};

    #[test]
    fn digest_covers_every_exact_public_package_fact() {
        let review = review();
        assert!(review.is_valid());
        for changed in [
            ProviderInstallReview {
                package: ProviderPackageReview {
                    package_name: "@openai/other".into(),
                    ..review.package.clone()
                },
                ..review.clone()
            },
            ProviderInstallReview {
                package: ProviderPackageReview {
                    version: "2.0.0".into(),
                    ..review.package.clone()
                },
                ..review.clone()
            },
            ProviderInstallReview {
                package: ProviderPackageReview {
                    integrity: "sha512-other".into(),
                    ..review.package.clone()
                },
                ..review.clone()
            },
            ProviderInstallReview {
                package: ProviderPackageReview {
                    package_policy_digest_sha256: "b".repeat(64),
                    ..review.package.clone()
                },
                ..review.clone()
            },
        ] {
            assert!(!changed.is_valid());
        }
    }

    #[test]
    fn malformed_public_artifact_is_not_valid() {
        let invalid = ProviderInstallReview {
            package: ProviderPackageReview {
                integrity: "latest".into(),
                ..review().package
            },
            ..review()
        };
        assert!(!invalid.is_valid());
    }

    fn review() -> ProviderInstallReview {
        ProviderInstallReview::reviewed(
            DependencyProvider::Codex,
            DependencyAction::Install,
            "Install the reviewed public provider package.",
            true,
            ProviderPackageReview {
                package_name: "@openai/codex".into(),
                version: "1.0.0".into(),
                integrity: "sha512-test".into(),
                package_policy_digest_sha256: "a".repeat(64),
            },
        )
    }
}
