//! Read-only three-provider onboarding projection derived from `gent doctor` facts.

use serde::{Deserialize, Serialize};

use crate::{CompatibilityTrust, DoctorReport, PrivateBridgeAvailability};

/// The complete and intentionally closed onboarding provider set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OnboardingProvider {
    Gent,
    Claude,
    Codex,
}

/// A non-mutating next state for a provider branch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OnboardingReadiness {
    Ready,
    ReviewCompatibility,
    ReviewInstallPlan,
    PrivateBridgeAvailable,
    PrivateBridgeUnavailable,
}

/// One renderable branch; actions remain separately consented protocol requests.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingBranch {
    pub provider: OnboardingProvider,
    pub readiness: OnboardingReadiness,
}

/// A read-only first-run view that never starts, downloads, or authenticates anything.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    pub branches: Vec<OnboardingBranch>,
}

impl OnboardingState {
    /// Derives exactly three branches from a previously read-only doctor report.
    #[must_use]
    pub fn from_doctor(doctor: &DoctorReport) -> Self {
        Self {
            branches: [
                OnboardingProvider::Gent,
                OnboardingProvider::Claude,
                OnboardingProvider::Codex,
            ]
            .into_iter()
            .map(|provider| OnboardingBranch {
                provider,
                readiness: readiness(doctor, provider),
            })
            .collect(),
        }
    }
}

fn readiness(doctor: &DoctorReport, provider: OnboardingProvider) -> OnboardingReadiness {
    match provider {
        OnboardingProvider::Gent => match doctor.private_bridge {
            PrivateBridgeAvailability::Available => OnboardingReadiness::PrivateBridgeAvailable,
            PrivateBridgeAvailability::NotConfigured => {
                OnboardingReadiness::PrivateBridgeUnavailable
            }
        },
        OnboardingProvider::Claude | OnboardingProvider::Codex => doctor
            .public_providers
            .iter()
            .find(|status| status.provider == provider_name(provider))
            .map_or(OnboardingReadiness::ReviewInstallPlan, |status| {
                if status.executable.is_none() {
                    OnboardingReadiness::ReviewInstallPlan
                } else if status.compatibility == CompatibilityTrust::Verified {
                    OnboardingReadiness::Ready
                } else {
                    OnboardingReadiness::ReviewCompatibility
                }
            }),
    }
}

const fn provider_name(provider: OnboardingProvider) -> &'static str {
    match provider {
        OnboardingProvider::Gent => "gent",
        OnboardingProvider::Claude => "claude",
        OnboardingProvider::Codex => "codex",
    }
}

#[cfg(test)]
mod tests {
    use super::{OnboardingProvider, OnboardingReadiness, OnboardingState};
    use crate::{
        CompatibilityTrust, DoctorReport, ExecutableIdentity, PrivateBridgeAvailability,
        PublicProviderStatus,
    };

    #[test]
    fn doctor_projection_is_closed_and_requires_explicit_public_install_review() {
        let state = OnboardingState::from_doctor(&DoctorReport::empty());
        assert_eq!(state.branches.len(), 3);
        assert_eq!(state.branches[0].provider, OnboardingProvider::Gent);
        assert_eq!(
            state.branches[0].readiness,
            OnboardingReadiness::PrivateBridgeUnavailable
        );
        assert!(
            state.branches[1..]
                .iter()
                .all(|branch| branch.readiness == OnboardingReadiness::ReviewInstallPlan)
        );
    }

    #[test]
    fn only_a_verified_public_identity_is_ready() {
        let mut doctor = DoctorReport::empty();
        doctor.private_bridge = PrivateBridgeAvailability::Available;
        doctor.public_providers.push(PublicProviderStatus {
            provider: "claude".into(),
            executable: Some(ExecutableIdentity {
                canonical_path: "/path/claude".into(),
                file_identity: "identity".into(),
                digest_sha256: "digest".into(),
                version: Some("1".into()),
            }),
            compatibility: CompatibilityTrust::Verified,
            remediation: "none".into(),
        });
        let state = OnboardingState::from_doctor(&doctor);
        assert_eq!(
            state.branches[0].readiness,
            OnboardingReadiness::PrivateBridgeAvailable
        );
        assert_eq!(state.branches[1].readiness, OnboardingReadiness::Ready);
        assert_eq!(
            state.branches[2].readiness,
            OnboardingReadiness::ReviewInstallPlan
        );
    }
}
