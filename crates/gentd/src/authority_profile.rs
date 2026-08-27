//! Validation seam for a future public-provider daemon composition.
//!
//! The shipped daemon never reads this configuration: it always composes the hard observer
//! service. A validated `PreparedPublicDrivers` value is deliberately only a preparation token;
//! it contains no resolver, runner, MCP client, Git executor, or automation scheduler. A later
//! composition must independently verify its evidence and compatibility material before using it.

#[cfg(test)]
#[path = "authority_profile_tests.rs"]
mod tests;

/// Requested authority surfaces before daemon composition.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AuthorityProfileConfig {
    /// Public Claude/Codex lifecycle authority, if separately approved.
    pub(crate) public_drivers: PublicDriverRequest,
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

impl ValidatedAuthorityProfile {
    /// Returns whether this profile has no authority beyond the shipped observer surface.
    ///
    /// Bootstrap uses this as a fail-closed regression fence: adding a future profile source must
    /// not accidentally make provider, MCP, Git, or automation authority reachable.
    #[must_use]
    pub(crate) const fn is_hard_observer(&self) -> bool {
        matches!(self, Self::Observer)
    }
}

impl AuthorityProfileConfig {
    /// Validates requested scope without resolving executables or creating external effects.
    ///
    /// # Errors
    /// Returns an error for incomplete public-driver approval or every deferred surface request.
    pub(crate) fn validate(self) -> Result<ValidatedAuthorityProfile, AuthorityProfileError> {
        reject_deferred("Git", self.git)?;
        reject_deferred("automations", self.automations)?;
        let public_drivers = match self.public_drivers {
            PublicDriverRequest::Disabled => None,
            PublicDriverRequest::Approved(approval) => {
                validate_approval(&approval)?;
                Some(approval)
            }
        };
        match public_drivers {
            None => Ok(ValidatedAuthorityProfile::Observer),
            Some(public_drivers) => Ok(ValidatedAuthorityProfile::PreparedPublicDrivers(
                public_drivers,
            )),
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
