//! Validation seam for a future public-provider daemon composition.
//!
//! The shipped daemon never reads this configuration: it always composes the hard observer
//! service. A validated `PreparedPublicDrivers` value is deliberately only a preparation token;
//! it contains no resolver, runner, MCP client, Git executor, or automation scheduler. A later
//! composition must independently verify its evidence and compatibility material before using it.

#[allow(dead_code)] // Deliberately dormant until MCP evidence gates are complete.
#[path = "mcp_authority.rs"]
mod mcp_authority;
#[cfg(test)]
#[path = "mcp_authority_tests.rs"]
mod mcp_authority_tests;
#[cfg(test)]
#[path = "authority_profile_tests.rs"]
mod tests;

/// Requested authority surfaces before daemon composition.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AuthorityProfileConfig {
    /// Public Claude/Codex lifecycle authority, if separately approved.
    pub(crate) public_drivers: PublicDriverRequest,
    /// MCP requires its own evidence-bound registry approval.
    pub(crate) mcp: McpRequest,
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

/// The only MCP request a future composition may prepare.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum McpRequest {
    /// The default: MCP cannot be resolved, connected, spawned, or inspected.
    #[default]
    Disabled,
    /// Preparation binds reviewed evidence to one immutable credential-free registry digest.
    #[allow(dead_code)] // Only a future reviewed composition may construct this request.
    Approved(McpApproval),
}

/// Identifies reviewed MCP evidence and the exact registry it approved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpApproval {
    pub(crate) evidence_reference: String,
    pub(crate) registry_sha256: String,
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
    #[error("MCP approval requires a non-empty evidence reference")]
    MissingMcpEvidenceReference,
    #[error("MCP approval requires a lowercase SHA-256 registry digest")]
    InvalidMcpRegistryDigest,
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
    PreparedMcp(McpApproval),
    PreparedPublicDriversAndMcp {
        public_drivers: PublicDriverApproval,
        mcp: McpApproval,
    },
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
        let mcp = match self.mcp {
            McpRequest::Disabled => None,
            McpRequest::Approved(approval) => {
                validate_mcp_approval(&approval)?;
                Some(approval)
            }
        };
        match (public_drivers, mcp) {
            (None, None) => Ok(ValidatedAuthorityProfile::Observer),
            (Some(public_drivers), None) => Ok(ValidatedAuthorityProfile::PreparedPublicDrivers(
                public_drivers,
            )),
            (None, Some(mcp)) => Ok(ValidatedAuthorityProfile::PreparedMcp(mcp)),
            (Some(public_drivers), Some(mcp)) => {
                Ok(ValidatedAuthorityProfile::PreparedPublicDriversAndMcp {
                    public_drivers,
                    mcp,
                })
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

fn validate_mcp_approval(approval: &McpApproval) -> Result<(), AuthorityProfileError> {
    if approval.evidence_reference.trim().is_empty() {
        return Err(AuthorityProfileError::MissingMcpEvidenceReference);
    }
    valid_sha256(&approval.registry_sha256)
        .then_some(())
        .ok_or(AuthorityProfileError::InvalidMcpRegistryDigest)
}

fn valid_sha256(value: &str) -> bool {
    let digest = value.as_bytes();
    digest.len() == 64
        && digest
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}
