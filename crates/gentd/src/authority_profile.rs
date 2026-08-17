//! Validation seam for a future public-provider daemon composition.
//!
//! The shipped daemon never reads this configuration: it always composes the hard observer
//! service. A validated `PreparedPublicDrivers` value is deliberately only a preparation token;
//! it contains no resolver, runner, MCP client, Git executor, or automation scheduler. A later
//! composition must independently verify its evidence and compatibility material before using it.

/// Requested authority surfaces before daemon composition.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AuthorityProfileConfig {
    /// Public Claude/Codex lifecycle authority, if separately approved.
    pub(crate) public_drivers: PublicDriverRequest,
    /// MCP spawning is not part of this composition seam.
    pub(crate) mcp: DeferredSurfaceRequest,
    /// Git mutation is not part of this composition seam.
    pub(crate) git: DeferredSurfaceRequest,
    /// Agent automations are not part of this composition seam.
    pub(crate) automations: DeferredSurfaceRequest,
}

/// The only public-provider lifecycle request accepted by this future seam.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum PublicDriverRequest {
    /// The default: no public provider can be resolved, launched, resumed, or interrupted.
    #[default]
    Disabled,
    /// Preparation requires a stable evidence reference and a pinned compatibility digest.
    #[allow(dead_code)] // Only a future reviewed composition may construct this request.
    Approved(PublicDriverApproval),
}

/// Identifies evidence already approved by a future composition owner.
///
/// This is not proof of approval. It makes the required proof explicit so a future composition
/// can load and revalidate it without accepting an uncorrelated boolean flag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublicDriverApproval {
    pub(crate) evidence_reference: String,
    pub(crate) compatibility_manifest_sha256: String,
}

/// A surface that this milestone cannot enable, even when requested explicitly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DeferredSurfaceRequest {
    #[default]
    Disabled,
    Requested,
}

/// Error returned before an unsupported authority surface becomes reachable.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum AuthorityProfileError {
    #[error("public-driver approval requires a non-empty evidence reference")]
    MissingEvidenceReference,
    #[error("public-driver approval requires a lowercase SHA-256 compatibility manifest digest")]
    InvalidCompatibilityDigest,
    #[error("{surface} authority is not composed in this daemon milestone")]
    DeferredSurfaceRequested { surface: &'static str },
}

/// Validated, non-effectful composition input.
///
/// `PreparedPublicDrivers` is intentionally not a runnable provider service. The main daemon
/// still selects `Observer`; this type only centralizes the checks a future composition must pass
/// before it may instantiate separately reviewed process and ingress adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ValidatedAuthorityProfile {
    Observer,
    PreparedPublicDrivers(PublicDriverApproval),
}

impl AuthorityProfileConfig {
    /// Validates requested scope without resolving executables or creating external effects.
    ///
    /// # Errors
    /// Returns an error for incomplete public-driver approval or every deferred surface request.
    pub(crate) fn validate(self) -> Result<ValidatedAuthorityProfile, AuthorityProfileError> {
        reject_deferred("MCP", self.mcp)?;
        reject_deferred("Git", self.git)?;
        reject_deferred("automations", self.automations)?;
        match self.public_drivers {
            PublicDriverRequest::Disabled => Ok(ValidatedAuthorityProfile::Observer),
            PublicDriverRequest::Approved(approval) => {
                validate_approval(&approval)?;
                Ok(ValidatedAuthorityProfile::PreparedPublicDrivers(approval))
            }
        }
    }
}

/// Validates the exact hard-observer profile shipped by `gentd`.
///
/// This calls no provider, MCP, Git, or automation code. It keeps the composition boundary
/// explicit at startup while no command-line or environment setting can select another profile.
#[must_use]
pub(crate) fn shipped_observer_profile() -> ValidatedAuthorityProfile {
    AuthorityProfileConfig::default()
        .validate()
        .expect("the built-in observer authority profile is valid")
}

fn reject_deferred(
    surface: &'static str,
    request: DeferredSurfaceRequest,
) -> Result<(), AuthorityProfileError> {
    if request == DeferredSurfaceRequest::Requested {
        return Err(AuthorityProfileError::DeferredSurfaceRequested { surface });
    }
    Ok(())
}

fn validate_approval(approval: &PublicDriverApproval) -> Result<(), AuthorityProfileError> {
    if approval.evidence_reference.trim().is_empty() {
        return Err(AuthorityProfileError::MissingEvidenceReference);
    }
    let digest = approval.compatibility_manifest_sha256.as_bytes();
    if digest.len() != 64
        || !digest
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(AuthorityProfileError::InvalidCompatibilityDigest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AuthorityProfileConfig, AuthorityProfileError, DeferredSurfaceRequest,
        PublicDriverApproval, PublicDriverRequest, ValidatedAuthorityProfile,
    };

    fn approval() -> PublicDriverApproval {
        PublicDriverApproval {
            evidence_reference: "public-driver-evidence-2026-08-16".into(),
            compatibility_manifest_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn default_profile_is_observer_and_all_deferred_surfaces_are_disabled() {
        let profile = AuthorityProfileConfig::default().validate().unwrap();
        assert_eq!(profile, ValidatedAuthorityProfile::Observer);
        assert!(matches!(profile, ValidatedAuthorityProfile::Observer));
    }

    #[test]
    fn public_drivers_need_explicit_complete_approval_but_do_not_compose_a_runner() {
        let incomplete = AuthorityProfileConfig {
            public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
                evidence_reference: " ".into(),
                compatibility_manifest_sha256: "a".repeat(64),
            }),
            ..AuthorityProfileConfig::default()
        };
        assert_eq!(
            incomplete.validate(),
            Err(AuthorityProfileError::MissingEvidenceReference)
        );
        let profile = AuthorityProfileConfig {
            public_drivers: PublicDriverRequest::Approved(approval()),
            ..AuthorityProfileConfig::default()
        }
        .validate()
        .unwrap();
        assert!(matches!(
            profile,
            ValidatedAuthorityProfile::PreparedPublicDrivers(_)
        ));
    }

    #[test]
    fn unsupported_surfaces_are_rejected_before_any_future_provider_composition() {
        let profile = AuthorityProfileConfig {
            public_drivers: PublicDriverRequest::Approved(approval()),
            git: DeferredSurfaceRequest::Requested,
            ..AuthorityProfileConfig::default()
        };
        assert_eq!(
            profile.validate(),
            Err(AuthorityProfileError::DeferredSurfaceRequested { surface: "Git" })
        );
    }
}
